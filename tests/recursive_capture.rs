#![cfg(unix)]
use avc::{
    Entry, Id, Object,
    capture::{CaptureMode, report},
    engine::Edit,
    repository::{CaptureScope, Error, Operation, Repository},
};
use std::{fs, path::Path, process::Command};

fn path(s: &str) -> Vec<Vec<u8>> {
    if s.is_empty() {
        Vec::new()
    } else {
        s.split('/').map(|p| p.as_bytes().to_vec()).collect()
    }
}
fn scope(directory: &Path, names: &[&str]) -> CaptureScope {
    CaptureScope::recursive(directory, names.iter().map(|p| path(p)).collect()).unwrap()
}
fn bytes(repo: &Repository, root: Id, name: &str) -> Vec<u8> {
    let engine = repo.load_root(root).unwrap();
    let Entry::File { blob, .. } = engine.read(root, &path(name)).unwrap() else {
        panic!("not a file");
    };
    let Object::Blob(data) = engine.object(blob).unwrap() else {
        panic!("not a blob");
    };
    data.clone()
}
fn cli(project: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_avc"))
        .arg("--repository")
        .arg(project)
        .args(args)
        .output()
        .unwrap()
}
fn success(project: &Path, args: &[&str]) -> String {
    let output = cli(project, args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn directory_selection_preserves_outside_source_head_and_excluded_native_bindings() {
    let project = tempfile::tempdir().unwrap();
    let input = tempfile::tempdir().unwrap();
    fs::create_dir(input.path().join("src")).unwrap();
    fs::write(input.path().join("src/a"), b"before").unwrap();
    fs::write(input.path().join("outside"), b"selected outside").unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let initial = repo
        .capture(
            &CaptureScope::new(input.path(), vec![path("src/a"), path("outside")]).unwrap(),
            0,
            "initial",
        )
        .unwrap();
    // Logical source can contain administration-like user data; exclusion must
    // preserve it, not silently delete it or read physical administration files.
    let mut engine = repo.load_root(initial.after.root).unwrap();
    let protected = Entry::File {
        blob: engine.blob(b"protected native bytes".to_vec()).unwrap(),
        executable: false,
    };
    let root = engine
        .edit(
            initial.after.root,
            &[Edit {
                path: path("src/.avc"),
                expected: Entry::Absent,
                value: protected,
            }],
        )
        .unwrap();
    repo.commit_source(&engine, root, 1, "protect").unwrap();
    let authored = repo
        .commit("A <a@example.com>", "before", 2, "author")
        .unwrap();
    fs::create_dir_all(input.path().join("src/.avc")).unwrap();
    fs::write(
        input.path().join("src/.avc/do-not-read"),
        b"not native source",
    )
    .unwrap();
    fs::create_dir(input.path().join("src/.git")).unwrap();
    fs::write(input.path().join("src/.git/HEAD"), b"not native source").unwrap();
    fs::create_dir(input.path().join("src/empty")).unwrap();
    fs::write(input.path().join("src/a"), b"after").unwrap();
    fs::write(input.path().join("src/new"), b"new").unwrap();
    fs::write(input.path().join("outside"), b"live outside edit").unwrap();
    let capture = repo
        .capture(&scope(input.path(), &["src"]), 3, "directory")
        .unwrap();
    assert_eq!(capture.after.revision, authored.after.revision);
    assert_eq!(bytes(&repo, capture.after.root, "src/a"), b"after");
    assert_eq!(bytes(&repo, capture.after.root, "src/new"), b"new");
    assert_eq!(
        bytes(&repo, capture.after.root, "outside"),
        b"selected outside"
    );
    assert_eq!(
        bytes(&repo, capture.after.root, "src/.avc"),
        b"protected native bytes"
    );
    let selected = repo.load_root(capture.after.root).unwrap();
    assert!(matches!(
        selected
            .read(capture.after.root, &path("src/empty"))
            .unwrap(),
        Entry::Directory(_)
    ));
    assert_eq!(
        selected
            .read(capture.after.root, &path("src/.git"))
            .unwrap(),
        Entry::Absent
    );
    let evidence = repo.capture_observation("directory").unwrap().unwrap();
    let coverage = report(&evidence).unwrap();
    assert_eq!(
        (coverage.files, coverage.symlinks, coverage.directories),
        (2, 0, 2)
    );
    assert_eq!(coverage.excluded, vec![path("src/.avc"), path("src/.git")]);
    let cancelled = repo.undo("directory", 4, "cancel").unwrap();
    assert_eq!(cancelled.after.root, authored.after.root);
    assert_eq!(cancelled.after.revision, authored.after.revision);
    assert_eq!(fs::read(input.path().join("src/a")).unwrap(), b"after");
    assert_eq!(
        fs::read(input.path().join("outside")).unwrap(),
        b"live outside edit"
    );
    assert_eq!(
        repo.capture_observation("directory").unwrap().unwrap(),
        evidence
    );
}

#[test]
fn recursive_retry_identity_includes_mode_and_recovers_before_any_filesystem_access() {
    let project = tempfile::tempdir().unwrap();
    let input = tempfile::tempdir().unwrap();
    fs::write(input.path().join("a"), b"original").unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let requested = scope(input.path(), &["a"]);
    let capture = repo.capture(&requested, 0, "recursive").unwrap();
    repo.commit("A <a@example.com>", "later", 1, "author")
        .unwrap();
    fs::remove_dir_all(input.path()).unwrap();
    let leaf = CaptureScope::new(input.path(), vec![path("a")]).unwrap();
    assert!(matches!(
        repo.capture(&leaf, 0, "recursive"),
        Err(Error::RequestKeyReuse(_))
    ));
    let later = repo.status().unwrap();
    assert_eq!(repo.capture(&requested, 0, "recursive").unwrap(), capture);
    assert_eq!(repo.status().unwrap(), later);
    drop(repo);
    let mut repo = Repository::open(project.path()).unwrap();
    assert_eq!(repo.capture(&requested, 0, "recursive").unwrap(), capture);
    let Operation::Capture { scope } = capture.operation else {
        panic!("not capture");
    };
    assert_eq!(scope.mode(), CaptureMode::Recursive);
}

#[test]
fn missing_selected_descendants_refuse_without_objects_witnesses_or_selection_effects() {
    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join("src")).unwrap();
    fs::write(project.path().join("src/a"), b"a").unwrap();
    fs::write(project.path().join("src/b"), b"b").unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let requested = scope(project.path(), &["src"]);
    let initial = repo.capture(&requested, 0, "initial").unwrap();
    fs::remove_file(project.path().join("src/b")).unwrap();
    fs::write(project.path().join("src/a"), b"not admitted").unwrap();
    let db = rusqlite::Connection::open(project.path().join(".avc/store.sqlite")).unwrap();
    let count = || {
        db.query_row::<i64, _, _>("SELECT count(*) FROM objects", [], |r| r.get(0))
            .unwrap()
    };
    let before = count();
    let error = repo
        .capture(&requested, 1, "missing")
        .unwrap_err()
        .to_string();
    assert!(error.contains("missing"), "{error}");
    assert_eq!(repo.status().unwrap(), initial.after);
    assert_eq!(count(), before);
    assert!(repo.receipt("missing").unwrap().is_none());
    assert!(repo.capture_observation("missing").unwrap().is_none());
    assert_eq!(bytes(&repo, initial.after.root, "src/b"), b"b");
    fs::remove_file(project.path().join("src/a")).unwrap();
    fs::remove_dir(project.path().join("src")).unwrap();
    fs::write(
        project.path().join("src"),
        b"not an implicit subtree deletion",
    )
    .unwrap();
    assert!(repo.capture(&requested, 1, "replace-directory").is_err());
    assert_eq!(repo.status().unwrap(), initial.after);
}

#[test]
fn actual_cli_recursive_root_reports_omissions_and_commits_pinned_bytes_only() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    drop(repo);
    fs::create_dir_all(project.path().join("src/empty")).unwrap();
    fs::create_dir(project.path().join(".git")).unwrap();
    fs::write(project.path().join(".git/HEAD"), b"not source").unwrap();
    fs::write(project.path().join("src/a"), b"preview").unwrap();
    let leaf = cli(
        project.path(),
        &[
            "--experimental",
            "capture",
            "--expected-generation",
            "0",
            "--request-key",
            "leaf",
            "src",
        ],
    );
    assert!(!leaf.status.success());
    let output = success(
        project.path(),
        &[
            "--experimental",
            "capture",
            "--recursive",
            "--expected-generation",
            "0",
            "--request-key",
            "root",
            ".",
        ],
    );
    assert!(output.contains("Capture mode: recursive"));
    assert!(output.contains("Captured: .\n"));
    assert!(output.contains("Observed: files=1 symlinks=0 directories=3"));
    assert!(output.contains("Unobserved administration scope: .avc"));
    assert!(output.contains("Unobserved administration scope: .git"));
    repo = Repository::open(project.path()).unwrap();
    let avc::repository::Receipt::Selection(receipt) = repo.receipt("root").unwrap().unwrap()
    else {
        panic!("expected selection receipt");
    };
    let captured = receipt.after.root;
    let Operation::Capture { scope: recorded } = &receipt.operation else {
        panic!("not capture");
    };
    assert_eq!(*recorded, scope(project.path(), &[""]));
    fs::write(project.path().join("src/a"), b"after preview").unwrap();
    success(
        project.path(),
        &[
            "--experimental",
            "commit",
            "--author",
            "A <a@example.com>",
            "--message",
            "pinned",
            "--expected-generation",
            "1",
            "--request-key",
            "author",
        ],
    );
    let authored = repo.status().unwrap();
    assert_eq!(
        repo.revision(authored.revision.unwrap()).unwrap().root,
        captured
    );
    assert_eq!(bytes(&repo, captured, "src/a"), b"preview");
    assert_eq!(
        fs::read(project.path().join("src/a")).unwrap(),
        b"after preview"
    );
    assert_eq!(
        repo.capture(&scope(project.path(), &[""]), 0, "root")
            .unwrap(),
        receipt
    );
}

#[test]
fn cli_normalizes_relative_scopes_but_refuses_escapes_and_overlapping_requests() {
    let project = tempfile::tempdir().unwrap();
    drop(Repository::init(project.path()).unwrap());
    fs::create_dir(project.path().join("src")).unwrap();
    fs::write(project.path().join("src/a"), b"a").unwrap();
    for requested in ["../escape", "/absolute", "src/../src", ""] {
        assert!(
            !cli(
                project.path(),
                &[
                    "--experimental",
                    "capture",
                    "--recursive",
                    "--expected-generation",
                    "0",
                    "--request-key",
                    "bad",
                    requested
                ]
            )
            .status
            .success()
        );
    }
    assert!(
        !cli(
            project.path(),
            &[
                "--experimental",
                "capture",
                "--recursive",
                "--expected-generation",
                "0",
                "--request-key",
                "overlap",
                ".",
                "src"
            ]
        )
        .status
        .success()
    );
    success(
        project.path(),
        &[
            "--experimental",
            "capture",
            "--recursive",
            "--expected-generation",
            "0",
            "--request-key",
            "good",
            "./src//",
        ],
    );
    let repo = Repository::open(project.path()).unwrap();
    let avc::repository::Receipt::Selection(receipt) = repo.receipt("good").unwrap().unwrap()
    else {
        panic!("expected selection receipt");
    };
    let Operation::Capture { scope: recorded } = receipt.operation else {
        panic!("not capture");
    };
    assert_eq!(recorded.paths(), &[path("src")]);
    assert!(repo.receipt("bad").unwrap().is_none());
    assert!(repo.receipt("overlap").unwrap().is_none());
}

#[test]
fn recovered_capture_ack_is_not_hidden_by_damaged_optional_report() {
    let project = tempfile::tempdir().unwrap();
    drop(Repository::init(project.path()).unwrap());
    fs::write(project.path().join("a"), b"a").unwrap();
    let args = [
        "--experimental",
        "capture",
        "--recursive",
        "--expected-generation",
        "0",
        "--request-key",
        "original",
        "a",
    ];
    success(project.path(), &args);
    let db = rusqlite::Connection::open(project.path().join(".avc/store.sqlite")).unwrap();
    db.execute_batch("DROP TRIGGER capture_observations_no_update; UPDATE capture_observations SET witnesses = X'00'").unwrap();
    fs::remove_file(project.path().join("a")).unwrap();
    let output = cli(project.path(), &args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("generation=1 request=original")
    );
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("Capture committed; observation diagnostics unavailable")
    );
    let repo = Repository::open(project.path()).unwrap();
    assert_eq!(repo.status().unwrap().generation, 1);
    assert!(repo.capture_observation("original").is_err());
}
