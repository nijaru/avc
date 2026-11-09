//! Real CLI process failures, NOT power-loss qualification. Runtime-compiled
//! interposition covers observed libc writes, syncs, creates
//! and truncates under one canonical temporary .avc. It does not emulate lost
//! kernel/device caches, mmap/SHM stores, unlink/close errors, or raw syscalls.
//! No production environment hooks or public test APIs are involved.
#![cfg(any(target_os = "linux", target_os = "macos"))]

use avc::{
    Entry, Id, Object,
    engine::{Edit, Engine},
    repository::{Receipt, Repository, Selection},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};
use tempfile::TempDir;

const KEY: &str = "fault-qualified-put";
const EXIT: i32 = 86;
const IO_ENV: [&str; 6] = [
    "AVC_IO_SCOPE",
    "AVC_IO_TRACE",
    "AVC_IO_MODE",
    "AVC_IO_NTH",
    "LD_PRELOAD",
    "DYLD_INSERT_LIBRARIES",
];

struct Harness {
    scratch: TempDir,
    library: PathBuf,
    probe: PathBuf,
}

impl Harness {
    fn new() -> Self {
        let scratch = tempfile::tempdir().unwrap();
        let library = scratch.path().join(if cfg!(target_os = "macos") {
            "faults.dylib"
        } else {
            "faults.so"
        });
        let probe = scratch.path().join("probe");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/io_faults.c");
        let harness = Self {
            scratch,
            library,
            probe,
        };
        for is_probe in [false, true] {
            let mut cc = Command::new("cc");
            clean(&mut cc);
            cc.args(["-std=c11", "-O2", "-Wall", "-Wextra", "-Werror"]);
            if is_probe {
                cc.arg("-DIO_FAULTS_PROBE");
            } else {
                cc.arg("-fPIC");
                cc.arg(if cfg!(target_os = "macos") {
                    "-dynamiclib"
                } else {
                    "-shared"
                });
            }
            cc.arg(&source).arg("-o").arg(if is_probe {
                &harness.probe
            } else {
                &harness.library
            });
            if !is_probe && cfg!(target_os = "linux") {
                cc.arg("-ldl");
            }
            let output = harness.run(&mut cc);
            assert_success(&output, "compile native fault shim/probe with system cc");
        }
        harness
    }

    // File-backed capture avoids pipe deadlock, including if a child emits more
    // than a pipe buffer. Every subprocess (including cc) has a kill+wait bound.
    fn run(&self, command: &mut Command) -> Output {
        let mut stdout = tempfile::tempfile_in(self.scratch.path()).unwrap();
        let mut stderr = tempfile::tempfile_in(self.scratch.path()).unwrap();
        let mut child = command
            .stdout(Stdio::from(stdout.try_clone().unwrap()))
            .stderr(Stdio::from(stderr.try_clone().unwrap()))
            .spawn()
            .unwrap_or_else(|error| panic!("spawn {command:?}: {error}"));
        let deadline = Instant::now() + Duration::from_secs(30);
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
                result => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("bounded subprocess did not finish: {command:?}: {result:?}");
                }
            }
        };
        let capture = |file: &mut File| {
            file.seek(SeekFrom::Start(0)).unwrap();
            let mut bytes = Vec::new();
            file.take(1_048_577).read_to_end(&mut bytes).unwrap();
            assert!(bytes.len() <= 1_048_576, "unbounded subprocess output");
            bytes
        };
        Output {
            status,
            stdout: capture(&mut stdout),
            stderr: capture(&mut stderr),
        }
    }

    fn cli(&self, project: &Path, args: &[&str]) -> Output {
        let mut command = cli(project, args);
        self.run(&mut command)
    }

    fn success(&self, project: &Path, args: &[&str]) -> Output {
        let output = self.cli(project, args);
        assert_success(&output, &format!("CLI {args:?}"));
        output
    }

    fn inject(
        &self,
        command: &mut Command,
        project: &Path,
        mode: &str,
        nth: usize,
    ) -> (Output, Vec<Event>) {
        let trace = tempfile::NamedTempFile::new_in(self.scratch.path()).unwrap();
        // Canonicalize on macOS too: /var and /tmp can be symlink aliases.
        command
            .env("AVC_IO_SCOPE", project.join(".avc").canonicalize().unwrap())
            .env("AVC_IO_TRACE", trace.path().canonicalize().unwrap())
            .env("AVC_IO_MODE", mode)
            .env("AVC_IO_NTH", nth.to_string())
            .env(
                if cfg!(target_os = "macos") {
                    "DYLD_INSERT_LIBRARIES"
                } else {
                    "LD_PRELOAD"
                },
                &self.library,
            );
        let output = self.run(command);
        assert_ne!(
            output.status.code(),
            Some(97),
            "shim ABI/path/logging failure: {command:?}\n{output:?}"
        );
        let events = fs::read_to_string(trace.path())
            .unwrap()
            .lines()
            .map(Event::parse)
            .collect::<Vec<_>>();
        for (index, event) in events.iter().enumerate() {
            assert_eq!(event.n, index + 1, "nonsequential trace: {events:?}");
        }
        (output, events)
    }
}

fn clean(command: &mut Command) {
    for key in IO_ENV {
        command.env_remove(key);
    }
}
fn cli(project: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_avc"));
    clean(&mut command);
    command.arg("--repository").arg(project).args(args);
    command
}
fn assert_success(output: &Output, context: &str) {
    assert!(
        output.status.success(),
        "{context}: {:?}\nstdout={}\nstderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
fn put(project: &Path, input: &Path) -> Command {
    cli(
        project,
        &[
            "--experimental",
            "source",
            "put",
            "file",
            "--file",
            input.to_str().unwrap(),
            "--expected-generation",
            "0",
            "--request-key",
            KEY,
        ],
    )
}
fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let destination = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &destination);
        } else {
            assert!(entry.file_type().unwrap().is_file());
            fs::copy(entry.path(), destination).unwrap();
        }
    }
}

#[derive(Debug)]
struct Event {
    n: usize,
    op: String,
    path: String,
    bytes: usize,
    offset: i64,
    action: String,
}
impl Event {
    fn parse(line: &str) -> Self {
        let fields: Vec<_> = line.split('\t').collect();
        assert_eq!(fields.len(), 6, "malformed native trace: {line}");
        Self {
            n: fields[0].parse().unwrap(),
            op: fields[1].into(),
            path: fields[2].into(),
            bytes: fields[3].parse().unwrap(),
            offset: fields[4].parse().unwrap(),
            action: fields[5].into(),
        }
    }
    fn signature(&self) -> (&str, &str, usize, i64) {
        (&self.op, &self.path, self.bytes, self.offset)
    }
    fn partial_eligible(&self) -> bool {
        matches!(self.op.as_str(), "write" | "pwrite" | "pwrite64") && self.bytes > 1
    }
}

#[test]
fn native_shim_scope_abi_and_short_write_contract() {
    use std::os::unix::fs::symlink;
    let h = Harness::new();
    let project = tempfile::tempdir().unwrap();
    let root = project.path().join(".avc");
    fs::create_dir(&root).unwrap();
    fs::create_dir(project.path().join(".avc-sibling")).unwrap();
    fs::create_dir(project.path().join("elsewhere")).unwrap();
    for path in ["outside", ".avc-sibling/file", "elsewhere/file"] {
        fs::write(project.path().join(path), b"original").unwrap();
    }
    symlink("../elsewhere", root.join("escape")).unwrap();
    symlink("../new-outside", root.join("dangling")).unwrap();
    let mut command = Command::new(&h.probe);
    clean(&mut command);
    command.arg("scope").arg(&root);
    let (output, trace) = h.inject(&mut command, project.path(), "eio", 0);
    assert_success(&output, &format!("native ABI and scope probe: {trace:?}"));
    let expected = if cfg!(target_os = "macos") {
        vec![
            "openat",
            "write",
            "pwrite",
            "fsync",
            "F_FULLFSYNC",
            "mkdirat",
            "ftruncate",
            "fsync",
        ]
    } else {
        vec![
            "openat",
            "write",
            "pwrite",
            "fsync",
            "mkdirat",
            "ftruncate",
            "fsync",
        ]
    };
    assert_eq!(
        trace.iter().map(|e| e.op.as_str()).collect::<Vec<_>>(),
        expected
    );
    assert!(trace.iter().all(|e| e.action == "pass"));
    assert_eq!(fs::read(root.join("probe")).unwrap(), b"abcXYfgh");
    assert_eq!(
        fs::metadata(root.join("probe"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    for path in ["outside", ".avc-sibling/file", "elsewhere/file"] {
        assert_eq!(fs::read(project.path().join(path)).unwrap(), b"safe!nal");
    }
    assert_eq!(
        fs::read(project.path().join("new-outside")).unwrap(),
        b"safe"
    );

    for (kind, op, offset, tail_offset) in [
        ("partial", "write", 0, 5),
        ("partial-pwrite", "pwrite", 7, 12),
    ] {
        fs::write(root.join("torn"), []).unwrap();
        let mut command = Command::new(&h.probe);
        clean(&mut command);
        command.arg(kind).arg(&root);
        let (output, trace) = h.inject(&mut command, project.path(), "partial-eio", 1);
        assert_success(&output, "short-write count/offset/errno probe");
        assert_eq!(trace.len(), 2);
        assert_eq!(trace[0].signature(), (op, "/torn", 10, offset));
        assert_eq!(trace[0].action, "partial:5");
        assert_eq!(trace[1].signature(), (op, "/torn", 5, tail_offset));
        assert_eq!(trace[1].action, "tail-error");
        let mut expected = vec![0; offset as usize];
        expected.extend_from_slice(b"01234");
        assert_eq!(fs::read(root.join("torn")).unwrap(), expected);
    }
}
use std::os::unix::fs::PermissionsExt;

fn intended(bytes: &[u8]) -> Id {
    // Independent candidate construction, not an authored revision, and not
    // trusting the faulted command's printed root or receipt as the oracle.
    let mut engine = Engine::new();
    let root = engine.empty().unwrap();
    let blob = engine.blob(bytes.to_vec()).unwrap();
    engine
        .edit(
            root,
            &[Edit {
                path: vec![b"file".to_vec()],
                expected: Entry::Absent,
                value: Entry::File {
                    blob,
                    executable: false,
                },
            }],
        )
        .unwrap()
}
fn check_selected(repository: &Repository, before: Selection, after: Selection, bytes: &[u8]) {
    let (selected, engine) = repository.load().unwrap();
    let receipt = repository.receipt(KEY).unwrap();
    assert!(selected == before || selected == after, "{selected:?}");
    assert!(matches!(
        engine.object(selected.root).unwrap(),
        Object::Map(_)
    ));
    if selected == before {
        assert_eq!(receipt, None);
        assert!(engine.list(selected.root, &[]).unwrap().is_empty());
    } else {
        assert_eq!(
            receipt,
            Some(Receipt {
                request_key: KEY.into(),
                before,
                after,
            })
        );
        let Entry::File { blob, executable } =
            engine.read(selected.root, &[b"file".to_vec()]).unwrap()
        else {
            panic!("selected source must contain exactly the intended file");
        };
        assert!(!executable);
        assert_eq!(engine.list(selected.root, &[]).unwrap().len(), 1);
        assert_eq!(engine.object(blob).unwrap(), &Object::Blob(bytes.to_vec()));
    }
}
fn operation_count(project: &Path) -> i64 {
    let db = rusqlite::Connection::open_with_flags(
        project.join(".avc/store.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    operations(&db)
}
fn operations(db: &rusqlite::Connection) -> i64 {
    db.query_row("SELECT count(*) FROM operations", [], |r| r.get(0))
        .unwrap()
}

fn cli_roundtrip(h: &Harness, project: &Path, root: Id, bytes: Option<&[u8]>) {
    let transfer = tempfile::tempdir().unwrap();
    let bundle = transfer.path().join("source.avc");
    h.success(
        project,
        &[
            "export",
            "--root",
            &root.to_string(),
            "--output",
            bundle.to_str().unwrap(),
        ],
    );
    import_restore(h, &bundle, root, bytes);
}

fn import_restore(h: &Harness, bundle: &Path, root: Id, bytes: Option<&[u8]>) {
    let transfer = tempfile::tempdir().unwrap();
    let destination = transfer.path().join("independent");
    fs::create_dir(&destination).unwrap();
    h.success(&destination, &["--experimental", "init"]);
    h.success(
        &destination,
        &[
            "--experimental",
            "import",
            "--input",
            bundle.to_str().unwrap(),
            "--request-key",
            "independent",
        ],
    );
    assert_eq!(
        Repository::open(&destination)
            .unwrap()
            .status()
            .unwrap()
            .root,
        root
    );
    h.success(&destination, &["verify"]);
    let fresh = transfer.path().canonicalize().unwrap().join("fresh");
    h.success(
        &destination,
        &["--experimental", "restore", "--to", fresh.to_str().unwrap()],
    );
    if let Some(bytes) = bytes {
        assert_eq!(h.success(&destination, &["show", "file"]).stdout, bytes);
        assert_eq!(fs::read(fresh.join("file")).unwrap(), bytes);
        assert_eq!(fs::read_dir(&fresh).unwrap().count(), 1);
    } else {
        assert_eq!(fs::read_dir(&fresh).unwrap().count(), 0);
    }
}

fn assert_faulted_prefix(
    baseline: &[Event],
    trace: &[Event],
    target: &Event,
    mode: &str,
    context: &str,
) {
    assert!(trace.len() >= target.n, "fault not reached: {context}");
    // Require the entire baseline prefix, not merely an event counter.
    for (expected, actual) in baseline.iter().zip(trace).take(target.n) {
        assert_eq!(actual.signature(), expected.signature(), "{context}");
    }
    let action = trace[target.n - 1].action.as_str();
    assert_eq!(
        action,
        if mode.starts_with("partial-") {
            format!("partial:{}", (target.bytes / 2).min(137))
        } else if mode == "exit" {
            "exit".into()
        } else {
            "error".into()
        },
        "{context}"
    );
}

fn export_command(project: &Path, output: &Path) -> Command {
    cli(project, &["export", "--output", output.to_str().unwrap()])
}

fn export_sweep(h: &Harness, seed: &Path, before: Selection, after: Selection, bytes: &[u8]) {
    let fixture = h.scratch.path().join("export-seed");
    copy_tree(seed, &fixture);
    // Output and its parent remain INSIDE the sole injection scope; ordinary
    // user files and independent transfer destinations are never targeted.
    fs::create_dir(fixture.join(".avc/exports")).unwrap();
    let baseline_project = h.scratch.path().join("export-baseline");
    copy_tree(&fixture, &baseline_project);
    let baseline_output = baseline_project.join(".avc/exports/source.avc");
    let (output, baseline) = h.inject(
        &mut export_command(&baseline_project, &baseline_output),
        &baseline_project,
        "eio",
        0,
    );
    assert_success(&output, "CLI export baseline");
    let bundle = fs::read(&baseline_output).unwrap();
    import_restore(h, &baseline_output, after.root, Some(bytes));
    let targets: Vec<_> = baseline
        .iter()
        .filter(|e| e.path == "/exports" || e.path.starts_with("/exports/"))
        .collect();
    assert!(!targets.is_empty() && targets.len() <= 128);
    assert!(
        targets
            .iter()
            .any(|e| matches!(e.op.as_str(), "open" | "open64" | "openat" | "openat64"))
    );
    assert!(
        targets
            .iter()
            .any(|e| matches!(e.op.as_str(), "write" | "pwrite" | "pwrite64"))
    );
    let syncs: Vec<_> = targets
        .iter()
        .filter(|e| matches!(e.op.as_str(), "fsync" | "fdatasync" | "F_FULLFSYNC"))
        .collect();
    assert!(
        syncs.len() >= 3,
        "file/parent/final-file syncs: {targets:?}"
    );
    assert_eq!(syncs.first().unwrap().path, "/exports/source.avc");
    assert_eq!(syncs.last().unwrap().path, "/exports/source.avc");
    assert!(syncs.iter().any(|e| e.path == "/exports"));
    if cfg!(target_os = "macos") {
        assert!(
            syncs
                .iter()
                .any(|e| e.op == "F_FULLFSYNC" && e.path == "/exports/source.avc")
        );
    }
    let mut counts = BTreeMap::new();
    for target in &targets {
        *counts
            .entry((target.op.as_str(), target.path.as_str()))
            .or_insert(0) += 1;
    }
    eprintln!(
        "export: {} total baseline events, {} output boundaries; {counts:?}",
        baseline.len(),
        targets.len()
    );
    let mut representatives = BTreeSet::new();
    for mode in ["eio", "enospc", "exit"] {
        for target in &targets {
            let case = tempfile::tempdir_in(h.scratch.path()).unwrap();
            let project = case.path().join("project");
            copy_tree(&fixture, &project);
            let path = project.join(".avc/exports/source.avc");
            let (output, trace) = h.inject(
                &mut export_command(&project, &path),
                &project,
                mode,
                target.n,
            );
            let context = format!("export {mode} {target:?}: {output:?}; {trace:?}");
            assert_faulted_prefix(&baseline, &trace, target, mode, &context);
            // Unlike optional SQLite cleanup, every selected export boundary is
            // required. In particular a complete bundle without checked file /
            // directory / final device flush is NOT a successful publication.
            assert!(
                !output.status.success(),
                "required export error swallowed: {context}"
            );
            assert!(
                output.stdout.is_empty(),
                "false export acknowledgment: {context}"
            );
            if mode == "exit" {
                assert_eq!(output.status.code(), Some(EXIT), "{context}");
            } else {
                assert_eq!(output.status.code(), Some(1), "{context}");
            }
            let repository = Repository::open(&project).unwrap();
            assert_eq!(repository.status().unwrap(), after, "{context}");
            check_selected(&repository, before, after, bytes);
            let mut retained = Vec::new();
            repository.export(after.root, &mut retained).unwrap();
            assert_eq!(retained, bundle, "acknowledged source changed: {context}");
            let restored = case
                .path()
                .canonicalize()
                .unwrap()
                .join("acknowledged-source");
            avc::materialize::restore(
                &repository.load_root(after.root).unwrap(),
                after.root,
                &restored,
            )
            .unwrap();
            assert_eq!(fs::read(restored.join("file")).unwrap(), bytes);
            drop(repository);
            if path.exists() {
                let partial = fs::read(&path).unwrap();
                assert!(bundle.starts_with(&partial), "{context}");
                assert!(
                    !h.run(&mut export_command(&project, &path)).status.success(),
                    "must refuse existing export: {context}"
                );
                assert_eq!(fs::read(&path).unwrap(), partial, "no clobber: {context}");
            }
            let retry = project.join(".avc/exports/retry.avc");
            assert_success(&h.run(&mut export_command(&project, &retry)), &context);
            assert_eq!(fs::read(&retry).unwrap(), bundle, "{context}");
            if representatives.insert((mode, target.op.as_str(), target.path.as_str())) {
                import_restore(h, &retry, after.root, Some(bytes));
            }
            assert_eq!(operation_count(&project), 1, "{context}");
            assert_eq!(
                fs::read(project.join("user.txt")).unwrap(),
                b"live user file"
            );
            assert_eq!(
                fs::read(project.join(".git/untouched")).unwrap(),
                b"foreign git"
            );
        }
    }
    eprintln!(
        "qualified {} required-output export fault cases",
        targets.len() * 3
    );
}

// Receipt is a recovery entry point too. Keep an idle SQLite connection alive
// in one fixture (no read transaction that would obstruct FULL checkpoint), and
// kill a real writer at its first database checkpoint write in the other.
fn receipt_wal_sweeps(
    h: &Harness,
    seed: &Path,
    input: &Path,
    expected: &Receipt,
    bytes: &[u8],
    source_baseline: &[Event],
    old_export: &[u8],
) {
    let alive = h.scratch.path().join("wal-alive-seed");
    copy_tree(seed, &alive);
    let keeper = wal_keeper(&alive);
    assert_success(&h.run(&mut put(&alive, input)), "prepare live WAL receipt");
    assert!(
        fs::metadata(alive.join(".avc/store.sqlite-wal"))
            .unwrap()
            .len()
            > 32
    );

    let recovered = h.scratch.path().join("wal-crashed-seed");
    copy_tree(seed, &recovered);
    let checkpoint = source_baseline
        .iter()
        .find(|e| {
            e.path == "/store.sqlite" && matches!(e.op.as_str(), "write" | "pwrite" | "pwrite64")
        })
        .expect("baseline must include a main-database checkpoint write");
    let (output, trace) = h.inject(
        &mut put(&recovered, input),
        &recovered,
        "exit",
        checkpoint.n,
    );
    assert_eq!(output.status.code(), Some(EXIT));
    assert_eq!(trace.last().unwrap().signature(), checkpoint.signature());
    // The CLI has already acknowledged generation 1; no opener has repaired or
    // checkpointed this crashed fixture before copies are made for each case.
    assert!(String::from_utf8_lossy(&output.stdout).contains("generation=1"));
    assert!(
        fs::metadata(recovered.join(".avc/store.sqlite-wal"))
            .unwrap()
            .len()
            > 32
    );

    let mut total = 0;
    for (name, fixture, live) in [
        ("wal-alive", &alive, true),
        ("wal-recovery", &recovered, false),
    ] {
        let baseline_project = h.scratch.path().join(format!("{name}-baseline"));
        copy_tree(fixture, &baseline_project);
        let holder = live.then(|| wal_keeper(&baseline_project));
        let (baseline_output, baseline) = h.inject(
            &mut cli(&baseline_project, &["receipt", KEY]),
            &baseline_project,
            "eio",
            0,
        );
        assert_success(&baseline_output, "receipt WAL baseline");
        assert!(!baseline.is_empty() && baseline.len() <= 256);
        assert!(baseline.iter().all(|e| e.action == "pass"));
        drop(holder);
        let mut counts = BTreeMap::new();
        for event in &baseline {
            *counts
                .entry((event.op.as_str(), event.path.as_str()))
                .or_insert(0) += 1;
        }
        eprintln!(
            "receipt {name}: {} observed events; {counts:?}",
            baseline.len()
        );
        for mode in ["eio", "enospc", "exit"] {
            for target in &baseline {
                total += 1;
                let case = tempfile::tempdir_in(h.scratch.path()).unwrap();
                let project = case.path().join("project");
                copy_tree(fixture, &project);
                let holder = live.then(|| wal_keeper(&project));
                let (output, trace) = h.inject(
                    &mut cli(&project, &["receipt", KEY]),
                    &project,
                    mode,
                    target.n,
                );
                let context = format!("receipt {name} {mode} {target:?}: {output:?}; {trace:?}");
                assert_faulted_prefix(&baseline, &trace, target, mode, &context);
                if mode == "exit" {
                    assert_eq!(output.status.code(), Some(EXIT), "{context}");
                } else {
                    assert!(output.status.code().is_some(), "{context}");
                }
                if output.status.success() {
                    assert_eq!(output.stdout, baseline_output.stdout, "{context}");
                }
                let repository =
                    Repository::open(&project).unwrap_or_else(|e| panic!("{e}: {context}"));
                assert_eq!(repository.status().unwrap(), expected.after, "{context}");
                assert_eq!(
                    repository.receipt(KEY).unwrap().as_ref(),
                    Some(expected),
                    "{context}"
                );
                check_selected(&repository, expected.before, expected.after, bytes);
                let mut old = Vec::new();
                repository.export(expected.before.root, &mut old).unwrap();
                assert_eq!(old, old_export, "{context}");
                drop(repository);
                for _ in 0..2 {
                    assert_eq!(
                        h.success(&project, &["receipt", KEY]).stdout,
                        baseline_output.stdout,
                        "{context}"
                    );
                }
                assert_success(&h.run(&mut put(&project, input)), &context);
                assert_eq!(operation_count(&project), 1, "{context}");
                assert_eq!(h.success(&project, &["show", "file"]).stdout, bytes);
                assert_eq!(
                    fs::read(project.join("user.txt")).unwrap(),
                    b"live user file"
                );
                assert_eq!(
                    fs::read(project.join(".git/untouched")).unwrap(),
                    b"foreign git"
                );
                drop(holder);
            }
        }
    }
    drop(keeper);
    eprintln!("qualified {total} additional receipt WAL fault cases");
}

fn wal_keeper(project: &Path) -> rusqlite::Connection {
    // An idle peer connection keeps the WAL file alive without holding a read
    // transaction that would obstruct the CLI's close-time FULL checkpoint.
    let connection = rusqlite::Connection::open(project.join(".avc/store.sqlite")).unwrap();
    operations(&connection);
    connection
}

#[test]
fn cli_observed_syscall_fault_sweep_preserves_source_and_exactly_once_retry() {
    let h = Harness::new();
    let seed = tempfile::tempdir().unwrap();
    fs::write(seed.path().join("user.txt"), b"live user file").unwrap();
    fs::create_dir(seed.path().join(".git")).unwrap();
    fs::write(seed.path().join(".git/untouched"), b"foreign git").unwrap();
    h.success(seed.path(), &["--experimental", "init"]);
    let before = Repository::open(seed.path()).unwrap().status().unwrap();
    assert_eq!(before.generation, 0);
    let old_export = {
        let repository = Repository::open(seed.path()).unwrap();
        let mut bytes = Vec::new();
        repository.export(before.root, &mut bytes).unwrap();
        bytes
    };
    let mut bytes = b"exact source\0\r\n".to_vec();
    bytes.extend((0..8192).map(|i| (i % 251) as u8));
    let input = h.scratch.path().join("input");
    fs::write(&input, &bytes).unwrap();
    let after = Selection {
        root: intended(&bytes),
        generation: 1,
    };
    let baseline_project = h.scratch.path().join("baseline");
    copy_tree(seed.path(), &baseline_project);
    let (baseline_output, baseline) = h.inject(
        &mut put(&baseline_project, &input),
        &baseline_project,
        "eio",
        0,
    );
    assert_success(&baseline_output, "traced baseline CLI put");
    assert!(
        !baseline.is_empty() && baseline.len() <= 256,
        "{baseline:?}"
    );
    assert!(baseline.iter().all(|e| e.action == "pass"));
    let repository = Repository::open(&baseline_project).unwrap();
    assert_eq!(repository.status().unwrap(), after);
    check_selected(&repository, before, after, &bytes);
    drop(repository);
    let ops: BTreeSet<_> = baseline.iter().map(|e| e.op.as_str()).collect();
    // Sanity-check interception without pinning the store layout: sweeps must
    // keep working if production moves to inline-only objects (no raw install).
    assert!(
        ops.iter()
            .any(|op| matches!(*op, "write" | "pwrite" | "pwrite64")),
        "must intercept modifying writes: {baseline:?}"
    );
    assert!(
        baseline.iter().any(|e| e.path.starts_with("/store.sqlite")),
        "must observe SQLite I/O: {baseline:?}"
    );
    assert!(ops.contains("fsync") || ops.contains("fdatasync"));
    if cfg!(target_os = "macos") {
        assert!(
            ops.contains("F_FULLFSYNC"),
            "must intercept macOS full flush: {baseline:?}"
        );
    }
    let mut counts = BTreeMap::new();
    for event in &baseline {
        *counts
            .entry((event.op.as_str(), event.path.as_str()))
            .or_insert(0) += 1;
    }
    eprintln!(
        "observed baseline: {} events; op/path counts: {counts:?}",
        baseline.len()
    );

    let mut representative = BTreeSet::new();
    let mut cases = 0;
    let mut successful_error_returns = 0;
    for mode in [
        "eio",
        "enospc",
        "exit",
        "partial-eio",
        "partial-enospc",
        "partial-exit",
    ] {
        for target in &baseline {
            if mode.starts_with("partial-") && !target.partial_eligible() {
                continue;
            }
            cases += 1;
            let case = tempfile::tempdir_in(h.scratch.path()).unwrap();
            let project = case.path().join("project");
            copy_tree(seed.path(), &project);
            let (output, trace) = h.inject(&mut put(&project, &input), &project, mode, target.n);
            let context = format!(
                "mode={mode}, target={target:?}, status={:?}, trace={trace:?}, stderr={}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
            assert_faulted_prefix(&baseline, &trace, target, mode, &context);
            if mode.ends_with("exit") {
                assert_eq!(output.status.code(), Some(EXIT), "{context}");
            } else {
                assert!(
                    output.status.code().is_some(),
                    "unexpected signal: {context}"
                );
                if mode.starts_with("partial-") {
                    assert!(
                        trace.iter().any(|e| e.action == "tail-error"),
                        "short write must be followed by real error: {context}"
                    );
                }
            }
            // All recovery is uninjected, on a fresh connection after the real
            // CLI has exited (including its SQLite close/checkpoint phase).
            let repository =
                Repository::open(&project).unwrap_or_else(|e| panic!("reopen: {e}; {context}"));
            check_selected(&repository, before, after, &bytes);
            assert!(
                output.stdout.is_empty() || output.stdout == baseline_output.stdout,
                "unexpected or torn acknowledgment: {context}"
            );
            if !output.stdout.is_empty() {
                assert_eq!(
                    repository.status().unwrap(),
                    after,
                    "lost acknowledged root: {context}"
                );
            }
            if output.status.success() {
                // SQLite can recover/fall back from a failed primitive, or
                // ignore optional close/checkpoint errors after committing.
                // An acknowledged command must nevertheless be exactly new.
                successful_error_returns += 1;
                assert_eq!(repository.status().unwrap(), after, "{context}");
                assert_eq!(output.stdout, baseline_output.stdout, "{context}");
            }
            assert_eq!(
                operation_count(&project),
                i64::from(repository.receipt(KEY).unwrap().is_some()),
                "{context}"
            );
            let mut exported = Vec::new();
            repository.export(before.root, &mut exported).unwrap();
            assert_eq!(
                exported, old_export,
                "old acknowledged root changed: {context}"
            );
            let old = repository.load_root(before.root).unwrap();
            let restored = case.path().canonicalize().unwrap().join("old-source");
            avc::materialize::restore(&old, before.root, &restored).unwrap();
            assert_eq!(fs::read_dir(restored).unwrap().count(), 0);
            // One representative of every observed op/path and failure mode
            // crosses CLI export/import/restore before retry, including any
            // committed-but-unacknowledged selected root.
            if representative.insert((mode, target.op.clone(), target.path.clone())) {
                cli_roundtrip(&h, &project, before.root, None);
                if repository.status().unwrap() == after {
                    cli_roundtrip(&h, &project, after.root, Some(&bytes));
                }
            }
            drop(repository);
            for _ in 0..2 {
                let retry = h.run(&mut put(&project, &input));
                assert_success(&retry, &context);
                assert_eq!(
                    retry.stdout, baseline_output.stdout,
                    "retry outcome: {context}"
                );
                let repository = Repository::open(&project).unwrap();
                assert_eq!(repository.status().unwrap(), after, "{context}");
                check_selected(&repository, before, after, &bytes);
                assert_eq!(operation_count(&project), 1, "duplicate effect: {context}");
            }
            assert_eq!(h.success(&project, &["show", "file"]).stdout, bytes);
            h.success(&project, &["receipt", KEY]);
            assert_eq!(
                fs::read(project.join("user.txt")).unwrap(),
                b"live user file"
            );
            assert_eq!(
                fs::read(project.join(".git/untouched")).unwrap(),
                b"foreign git"
            );
            assert_eq!(fs::read(&input).unwrap(), bytes);
        }
    }
    cli_roundtrip(&h, &baseline_project, after.root, Some(&bytes));
    export_sweep(&h, &baseline_project, before, after, &bytes);
    receipt_wal_sweeps(
        &h,
        seed.path(),
        &input,
        &Receipt {
            request_key: KEY.into(),
            before,
            after,
        },
        &bytes,
        &baseline,
        &old_export,
    );
    eprintln!(
        "qualified {cases} fault cases; {successful_error_returns} injected-error commands still acknowledged a coherent committed result; {} representative op/path/mode CLI transfers",
        representative.len()
    );
}
