//! Stock Git is the independent oracle for object framing, trees and ancestry.
use avc::{
    Entry, Id, Object,
    engine::{Edit, Engine, MergeBudget},
    git::GitOid,
    repository::{Error, Operation, Repository},
};
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

fn git(dir: &Path, args: &[&str], input: &[u8]) -> Vec<u8> {
    let mut child = Command::new("git")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .current_dir(dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}
fn text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap().trim().to_owned()
}
fn hash(dir: &Path, kind: &str, bytes: &[u8]) -> String {
    text(git(
        dir,
        &["hash-object", "-w", "-t", kind, "--stdin"],
        bytes,
    ))
}
fn objects(dir: &Path) -> BTreeSet<String> {
    text(git(
        dir,
        &[
            "cat-file",
            "--batch-all-objects",
            "--batch-check=%(objectname)",
        ],
        b"",
    ))
    .lines()
    .map(str::to_owned)
    .collect()
}
fn counts(dir: &Path) -> (i64, i64, i64) {
    let db = rusqlite::Connection::open(dir.join(".avc/store.sqlite")).unwrap();
    db.query_row("SELECT (SELECT count(*) FROM objects), (SELECT count(*) FROM git_origins), (SELECT count(*) FROM operations)", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap()
}
fn commit_bytes(tree: &str, parent: Option<&str>, message: &[u8]) -> Vec<u8> {
    let mut bytes = format!("tree {tree}\n{}author Original <author@example.test> 1700000000 -0230\ncommitter Collector <collector@example.test> 1700000001 +0545\nencoding UTF-8\n\n", parent.map(|id| format!("parent {id}\n")).unwrap_or_default()).into_bytes();
    bytes.extend_from_slice(message);
    bytes
}
struct Fixture {
    _temp: tempfile::TempDir,
    git: PathBuf,
    commit: GitOid,
    raw: Vec<u8>,
    entries: Vec<(Vec<u8>, bool, bool, Vec<u8>)>,
    listing: Vec<u8>,
    tree: String,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("foreign.git");
        git(temp.path(), &["init", "--bare", dir.to_str().unwrap()], b"");
        let entries = vec![
            (b"README".to_vec(),false,false,b"line one\r\nline two\n".to_vec()),
            (b"bin".to_vec(),false,false,vec![0,255,1,13,10]),
            (b"run".to_vec(),true,false,b"#!/bin/sh\nexit 0\n".to_vec()),
            (b"link".to_vec(),false,true,b"README".to_vec()),
            (b"lfs".to_vec(),false,false,b"version https://git-lfs.github.com/spec/v1\noid sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nsize 123456789\n".to_vec()),
            (vec![255],false,false,b"raw name".to_vec()),
            (b"a.c".to_vec(),false,false,b"directory-prefix sort".to_vec()),
        ];
        let mut listing = Vec::new();
        for (name, exec, link, bytes) in &entries {
            let oid = hash(&dir, "blob", bytes);
            listing.extend_from_slice(
                format!(
                    "{} blob {oid}\t",
                    if *link {
                        "120000"
                    } else if *exec {
                        "100755"
                    } else {
                        "100644"
                    }
                )
                .as_bytes(),
            );
            listing.extend_from_slice(name);
            listing.push(0);
        }
        let nested = hash(&dir, "blob", b"nested");
        let subtree = text(git(
            &dir,
            &["mktree", "-z"],
            format!("100644 blob {nested}\tnested\0").as_bytes(),
        ));
        listing.extend_from_slice(format!("040000 tree {subtree}\ta\0").as_bytes());
        let tree = text(git(&dir, &["mktree", "-z"], &listing));
        let base_tree = text(git(&dir, &["mktree"], b""));
        let parent = hash(
            &dir,
            "commit",
            &commit_bytes(&base_tree, None, b"older Git history\n"),
        );
        let raw = commit_bytes(
            &tree,
            Some(&parent),
            "import \u{03bb}\n\nExact terminal bytes".as_bytes(),
        );
        let commit: GitOid = hash(&dir, "commit", &raw).parse().unwrap();
        git(
            &dir,
            &[
                "update-ref",
                "refs/heads/main",
                &commit.to_string().replace("sha1:", ""),
            ],
            b"",
        );
        Self {
            _temp: temp,
            git: dir,
            commit,
            raw,
            entries,
            listing,
            tree,
        }
    }
    fn native(&self) -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("live"), b"untouched").unwrap();
        let mut repo = Repository::init(dir.path()).unwrap();
        repo.import_git(&self.git, self.commit, "origin").unwrap();
        (dir, repo)
    }
}
fn put(repo: &mut Repository, key: &str, name: &[u8], bytes: &[u8]) {
    let (selected, mut engine) = repo.load().unwrap();
    let path = vec![name.to_vec()];
    let expected = engine.read(selected.root, &path).unwrap();
    let blob = engine.blob(bytes.to_vec()).unwrap();
    let root = engine
        .edit(
            selected.root,
            &[Edit {
                path,
                expected,
                value: Entry::File {
                    blob,
                    executable: false,
                },
            }],
        )
        .unwrap();
    repo.commit_source(&engine, root, selected.generation, key)
        .unwrap();
}
fn author(repo: &mut Repository, who: &str, key: &str) -> Id {
    repo.commit(who, key, repo.status().unwrap().generation, key)
        .unwrap()
        .after
        .revision
        .unwrap()
}
#[test]
fn deterministic_anchor_and_native_children_roundtrip_with_exact_foreign_parent() {
    let f = Fixture::new();
    let refs = git(
        &f.git,
        &["for-each-ref", "--format=%(refname) %(objectname)"],
        b"",
    );
    let (dir, mut repo) = f.native();
    let selected = repo.status().unwrap();
    let anchor = selected.revision.unwrap();
    let origin = repo.git_origin(anchor).unwrap().unwrap();
    assert_eq!(origin.raw_commit, f.raw);
    assert_eq!(origin.commit, f.commit);
    assert_eq!(origin.parents.len(), 1);
    let revision = repo.revision(anchor).unwrap();
    assert!(revision.parents.is_empty());
    assert_eq!(revision.author, "Original <author@example.test>");
    assert_eq!(revision.time_ns, 1_700_000_001_000_000_000);
    assert_eq!(revision.timezone_minutes, 345);
    assert_eq!(revision.message, "import \u{03bb}\n\nExact terminal bytes");
    let mut input = b"AVC-GIT-ANCHOR\0v1\0".to_vec();
    input.extend_from_slice(&[1, 20]);
    input.extend_from_slice(&f.commit.to_bytes()[1..]);
    assert_eq!(revision.change, &blake3::hash(&input).as_bytes()[..16]);
    let (_, engine) = repo.load().unwrap();
    let mut independent = Engine::new();
    let empty = independent.empty().unwrap();
    let mut edits = Vec::new();
    for (name, exec, link, bytes) in &f.entries {
        let entry = engine
            .read(selected.root, std::slice::from_ref(name))
            .unwrap();
        let blob = match entry {
            Entry::File { blob, executable } => {
                assert_eq!(executable, *exec);
                assert!(!link);
                blob
            }
            Entry::Symlink(blob) => {
                assert!(*link);
                blob
            }
            _ => panic!("wrong kind"),
        };
        assert_eq!(engine.object(blob).unwrap(), &Object::Blob(bytes.clone()));
        let blob = independent.blob(bytes.clone()).unwrap();
        edits.push(Edit {
            path: vec![name.clone()],
            expected: Entry::Absent,
            value: if *link {
                Entry::Symlink(blob)
            } else {
                Entry::File {
                    blob,
                    executable: *exec,
                }
            },
        });
    }
    let nested_blob = independent.blob(b"nested".to_vec()).unwrap();
    let subtree = independent
        .edit(
            empty,
            &[Edit {
                path: vec![b"nested".to_vec()],
                expected: Entry::Absent,
                value: Entry::File {
                    blob: nested_blob,
                    executable: false,
                },
            }],
        )
        .unwrap();
    edits.push(Edit {
        path: vec![b"a".to_vec()],
        expected: Entry::Absent,
        value: Entry::Directory(subtree),
    });
    assert_eq!(independent.edit(empty, &edits).unwrap(), selected.root);
    let (other, other_repo) = f.native();
    assert_eq!(other_repo.status().unwrap(), selected);
    drop(other_repo);
    drop(other);
    assert!(
        repo.amend("Native <native@example.test>", "bad", 1, "anchor-amend")
            .is_err()
    );
    assert!(repo.rebase(anchor, 1, "anchor-rebase").is_err());
    assert_eq!(repo.status().unwrap(), selected);
    put(&mut repo, "edit", b"native", b"native\t\r\n");
    let child = author(&mut repo, "Native <native@example.test>", "child");
    let before = repo.status().unwrap();
    let candidate = repo.export_git(&f.git, child).unwrap();
    let hex = candidate.to_string().replace("sha1:", "");
    let original = f.commit.to_string().replace("sha1:", "");
    assert_eq!(
        text(git(
            &f.git,
            &["rev-list", "--parents", "-n", "1", &hex],
            b""
        )),
        format!("{hex} {original}")
    );
    assert_eq!(
        git(&f.git, &["show", &format!("{hex}:native")], b""),
        b"native\t\r\n"
    );
    let new_blob = hash(&f.git, "blob", b"native\t\r\n");
    let mut expected_tree = f.listing.clone();
    expected_tree.extend_from_slice(format!("100644 blob {new_blob}\tnative\0").as_bytes());
    assert_eq!(
        text(git(&f.git, &["rev-parse", &format!("{hex}^{{tree}}")], b"")),
        text(git(&f.git, &["mktree", "-z"], &expected_tree))
    );
    let raw = git(&f.git, &["cat-file", "commit", &hex], b"");
    let authored = repo.revision(child).unwrap();
    let reimported_dir = tempfile::tempdir().unwrap();
    let mut reimported = Repository::init(reimported_dir.path()).unwrap();
    let imported = reimported
        .import_git(&f.git, candidate, "foreign-audit-headers")
        .unwrap();
    let new_anchor = imported.after.revision.unwrap();
    assert_ne!(new_anchor, child);
    assert_eq!(imported.after.root, authored.root);
    let converted = reimported.revision(new_anchor).unwrap();
    assert!(converted.parents.is_empty());
    assert_ne!(converted.change, authored.change);
    assert_eq!(
        converted.time_ns,
        authored.time_ns.div_euclid(1_000_000_000) * 1_000_000_000
    );
    assert!(matches!(
        reimported.revision(child),
        Err(Error::MissingObject(_))
    ));
    assert!(
        raw.windows(format!("avc-revision {child}\n").len())
            .any(|s| s == format!("avc-revision {child}\n").as_bytes())
    );
    assert!(String::from_utf8_lossy(&raw).contains(&format!("avc-time-ns {}\n", authored.time_ns)));
    git(&f.git, &["fsck", "--no-dangling", &hex], b"");
    assert_eq!(repo.export_git(&f.git, child).unwrap(), candidate);
    assert_eq!(repo.status().unwrap(), before);
    drop(repo);
    let mut repo = Repository::open(dir.path()).unwrap();
    assert_eq!(repo.export_git(&f.git, child).unwrap(), candidate);
    let second = author(&mut repo, "Native <native@example.test>", "second");
    let second_git = repo
        .export_git(&f.git, second)
        .unwrap()
        .to_string()
        .replace("sha1:", "");
    assert_eq!(
        text(git(&f.git, &["rev-parse", &format!("{second_git}^")], b"")),
        hex
    );
    assert_eq!(
        git(
            &f.git,
            &["for-each-ref", "--format=%(refname) %(objectname)"],
            b""
        ),
        refs
    );
    assert_eq!(fs::read(dir.path().join("live")).unwrap(), b"untouched");
    let output = dir.path().join("history.avc");
    assert!(repo.export_history_file(&output).is_err());
    assert!(!output.exists());
    let mut source = Vec::new();
    repo.export(repo.status().unwrap().root, &mut source)
        .unwrap();
    assert!(source.starts_with(b"AVCEXP01"));
    let db = rusqlite::Connection::open(dir.path().join(".avc/store.sqlite")).unwrap();
    assert_eq!(
        db.query_row::<i64, _, _>("SELECT count(*) FROM git_exports", [], |r| r.get(0))
            .unwrap(),
        2
    );
    fs::rename(&f.git, f.git.with_extension("gone")).unwrap();
    let prior = repo.import_git(&f.git, f.commit, "origin").unwrap();
    assert_eq!(prior.operation, Operation::ImportGit { commit: f.commit });
    assert_eq!(prior.after, selected);
    assert!(matches!(
        repo.import_git(
            &f.git,
            "0000000000000000000000000000000000000001".parse().unwrap(),
            "origin"
        ),
        Err(Error::RequestKeyReuse(_))
    ));
}

#[test]
fn import_refusals_do_not_admit_objects_or_select_an_anchor() {
    for case in [
        "gitlink",
        "nonutf8",
        "missing-blob",
        "corrupt-blob",
        "wrong-kind",
        "sha256",
    ] {
        let f = Fixture::new();
        let dir = tempfile::tempdir().unwrap();
        let mut repo = Repository::init(dir.path()).unwrap();
        let before = repo.status().unwrap();
        let initial = counts(dir.path());
        let mut commit = f.commit;
        let mut foreign = f.git.clone();
        match case {
            "gitlink" => {
                let body = format!(
                    "160000 commit {}\tsubmodule\n",
                    f.commit.to_string().replace("sha1:", "")
                );
                let tree = text(git(&f.git, &["mktree"], body.as_bytes()));
                commit = hash(&f.git, "commit", &commit_bytes(&tree, None, b"unsupported"))
                    .parse()
                    .unwrap();
            }
            "nonutf8" => {
                commit = hash(&f.git, "commit", &commit_bytes(&f.tree, None, &[255]))
                    .parse()
                    .unwrap()
            }
            "missing-blob" | "corrupt-blob" => {
                let oid = hash(&f.git, "blob", &f.entries[0].3);
                let path = f.git.join("objects").join(&oid[..2]).join(&oid[2..]);
                if case == "missing-blob" {
                    fs::remove_file(path).unwrap();
                } else {
                    let wrong = hash(&f.git, "blob", b"different valid compressed object");
                    // Git installs loose objects read-only. Replace this owned
                    // fixture inode rather than depending on write permissions.
                    fs::remove_file(&path).unwrap();
                    fs::copy(
                        f.git.join("objects").join(&wrong[..2]).join(&wrong[2..]),
                        path,
                    )
                    .unwrap();
                }
            }
            "wrong-kind" => commit = hash(&f.git, "blob", b"not a commit").parse().unwrap(),
            "sha256" => {
                foreign = dir.path().join("sha256.git");
                git(
                    dir.path(),
                    &[
                        "init",
                        "--bare",
                        "--object-format=sha256",
                        foreign.to_str().unwrap(),
                    ],
                    b"",
                );
            }
            _ => unreachable!(),
        }
        assert!(repo.import_git(&foreign, commit, "bad").is_err(), "{case}");
        assert_eq!(repo.status().unwrap(), before, "{case}");
        assert_eq!(counts(dir.path()), initial, "{case}");
        assert!(repo.receipt("bad").unwrap().is_none());
    }
}

#[test]
fn nonrepresentable_exports_preflight_everything_before_any_git_writes() {
    for case in ["empty-dir", "conflict", "identity"] {
        let f = Fixture::new();
        let (dir, mut repo) = f.native();
        put(&mut repo, "new", b"new-unwritten-blob", case.as_bytes());
        if case != "identity" {
            let (selected, mut engine) = repo.load().unwrap();
            let root = if case == "empty-dir" {
                let empty = engine.empty().unwrap();
                engine
                    .edit(
                        selected.root,
                        &[Edit {
                            path: vec![b"empty".to_vec()],
                            expected: Entry::Absent,
                            value: Entry::Directory(empty),
                        }],
                    )
                    .unwrap()
            } else {
                let path = vec![b"README".to_vec()];
                let expected = engine.read(selected.root, &path).unwrap();
                let mut roots = Vec::new();
                for bytes in [vec![0, 1], vec![0, 2]] {
                    let blob = engine.blob(bytes).unwrap();
                    roots.push(
                        engine
                            .edit(
                                selected.root,
                                &[Edit {
                                    path: path.clone(),
                                    expected: expected.clone(),
                                    value: Entry::File {
                                        blob,
                                        executable: false,
                                    },
                                }],
                            )
                            .unwrap(),
                    );
                }
                engine
                    .merge(selected.root, roots[0], roots[1], MergeBudget::default())
                    .unwrap()
                    .root
            };
            repo.commit_source(&engine, root, selected.generation, "unsupported-source")
                .unwrap();
        }
        let head = author(
            &mut repo,
            if case == "identity" {
                "No email"
            } else {
                "Native <native@example.test>"
            },
            "unsupported",
        );
        let present = objects(&f.git);
        let before = repo.status().unwrap();
        let count = counts(dir.path());
        assert!(repo.export_git(&f.git, head).is_err(), "{case}");
        assert_eq!(objects(&f.git), present, "{case}");
        assert_eq!(repo.status().unwrap(), before);
        assert_eq!(counts(dir.path()), count);
    }
}

#[test]
fn origin_and_receipt_publication_rollback_together_and_corruption_never_rebinds() {
    let f = Fixture::new();
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(dir.path()).unwrap();
    let db = rusqlite::Connection::open(dir.path().join(".avc/store.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER deny_git_receipt BEFORE INSERT ON operations WHEN NEW.kind = 'import-git' BEGIN SELECT RAISE(ABORT,'denied'); END;").unwrap();
    let before = repo.status().unwrap();
    let initial = counts(dir.path());
    assert!(repo.import_git(&f.git, f.commit, "origin").is_err());
    assert_eq!(repo.status().unwrap(), before);
    assert_eq!(counts(dir.path()), initial);
    db.execute_batch("DROP TRIGGER deny_git_receipt;").unwrap();
    let imported = repo.import_git(&f.git, f.commit, "origin").unwrap();
    let anchor = imported.after.revision.unwrap();
    let old = repo.undo("origin", 1, "undo").unwrap();
    assert_eq!(old.after.root, before.root);
    assert_eq!(old.after.revision, None);
    assert!(repo.git_origin(anchor).unwrap().is_some());
    assert_eq!(
        repo.import_git(&f.git, f.commit, "origin").unwrap(),
        imported
    );
    assert!(matches!(
        repo.import_git(&f.git, f.commit, "new-key"),
        Err(Error::NotEmpty)
    ));
    repo.undo("undo", 2, "redo").unwrap();
    put(&mut repo, "edit", b"new", b"new data");
    let head = author(&mut repo, "Native <native@example.test>", "child");
    db.execute_batch("DROP TRIGGER git_origins_no_update; UPDATE git_origins SET raw_commit = CAST(raw_commit || X'78' AS BLOB);").unwrap();
    let present = objects(&f.git);
    assert!(repo.git_origin(anchor).is_err());
    assert!(repo.export_git(&f.git, head).is_err());
    assert_eq!(objects(&f.git), present);
    // ACK recovery is not an assertion that retained storage is still healthy.
    assert_eq!(
        repo.import_git(&f.git, f.commit, "origin").unwrap(),
        imported
    );
}

#[test]
fn missing_origins_and_changed_derived_mappings_refuse_without_git_effects() {
    for missing in [true, false] {
        let f = Fixture::new();
        let (dir, mut repo) = f.native();
        let imported = repo.receipt("origin").unwrap().unwrap();
        let first = author(&mut repo, "Native <native@example.test>", "first");
        repo.export_git(&f.git, first).unwrap();
        put(
            &mut repo,
            "later-source",
            b"never-written-to-git",
            b"new Git payload",
        );
        let head = author(&mut repo, "Native <native@example.test>", "later");
        let db = rusqlite::Connection::open(dir.path().join(".avc/store.sqlite")).unwrap();
        if missing {
            db.execute_batch("PRAGMA foreign_keys=OFF; DROP TRIGGER git_origins_no_delete; DELETE FROM git_origins;")
                .unwrap();
            assert!(repo.git_origins(head).is_err());
            let output = dir.path().join("cannot-drop-boundary.avc");
            assert!(repo.export_history_file(&output).is_err());
            assert!(!output.exists());
            assert_eq!(
                repo.import_git(&f.git, f.commit, "origin").unwrap(),
                imported
            );
        } else {
            db.execute_batch("DROP TRIGGER git_exports_no_update;")
                .unwrap();
            let mut wrong = vec![9; 21];
            wrong[0] = 1;
            db.execute("UPDATE git_exports SET commit_oid = ?1", [&wrong])
                .unwrap();
        }
        let present = objects(&f.git);
        let selected = repo.status().unwrap();
        assert!(repo.export_git(&f.git, head).is_err());
        assert_eq!(objects(&f.git), present);
        assert_eq!(repo.status().unwrap(), selected);
        repo.load_root(head).unwrap(); // canonical native data is unaffected
    }
}

#[test]
fn concurrent_import_reconciles_identical_requests_and_keeps_one_origin() {
    for same_key in [true, false] {
        let f = Fixture::new();
        let dir = tempfile::tempdir().unwrap();
        drop(Repository::init(dir.path()).unwrap());
        let handles = [
            Repository::open(dir.path()).unwrap(),
            Repository::open(dir.path()).unwrap(),
        ];
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let tasks: Vec<_> = handles
            .into_iter()
            .enumerate()
            .map(|(n, mut repo)| {
                let barrier = barrier.clone();
                let git = f.git.clone();
                let commit = f.commit;
                std::thread::spawn(move || {
                    barrier.wait();
                    repo.import_git(&git, commit, if same_key || n == 0 { "one" } else { "two" })
                })
            })
            .collect();
        let results: Vec<_> = tasks.into_iter().map(|task| task.join().unwrap()).collect();
        if same_key {
            assert_eq!(results[0].as_ref().unwrap(), results[1].as_ref().unwrap());
        } else {
            assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
            assert!(results.iter().any(|r| matches!(r, Err(Error::NotEmpty))));
        }
        let repo = Repository::open(dir.path()).unwrap();
        let anchor = repo.status().unwrap().revision.unwrap();
        assert_eq!(
            counts(dir.path()),
            (
                repo.load_root(anchor).unwrap().object_count() as i64 + 1,
                1,
                1
            )
        );
    }
}

#[test]
fn missing_foreign_ancestors_are_frontiers_not_fake_native_parents() {
    let f = Fixture::new();
    let parent = String::from_utf8(f.raw.clone())
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("parent "))
        .unwrap()
        .to_owned();
    fs::remove_file(f.git.join("objects").join(&parent[..2]).join(&parent[2..])).unwrap();
    let (_dir, mut repo) = f.native();
    let anchor = repo.status().unwrap().revision.unwrap();
    assert!(repo.revision(anchor).unwrap().parents.is_empty());
    assert_eq!(
        repo.git_origins(anchor).unwrap()[0].parents,
        vec![parent.parse().unwrap()]
    );
    let child = author(&mut repo, "Native <native@example.test>", "empty child");
    let candidate = repo
        .export_git(&f.git, child)
        .unwrap()
        .to_string()
        .replace("sha1:", "");
    let raw = git(&f.git, &["cat-file", "commit", &candidate], b"");
    assert!(String::from_utf8(raw).unwrap().contains(&format!(
        "parent {}\n",
        f.commit.to_string().replace("sha1:", "")
    )));
}

#[test]
fn actual_cli_roundtrip_guards_mutations_and_reports_foreign_scope() {
    fn cli(dir: &Path, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_avc"))
            .arg("--repository")
            .arg(dir)
            .args(args)
            .output()
            .unwrap()
    }
    fn success(output: std::process::Output) -> String {
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
    let f = Fixture::new();
    let dir = tempfile::tempdir().unwrap();
    success(cli(dir.path(), &["--experimental", "init"]));
    let foreign = f.git.to_str().unwrap();
    let commit = f.commit.to_string();
    let args = [
        "git",
        "import",
        "--git-dir",
        foreign,
        "--commit",
        &commit,
        "--request-key",
        "cli-origin",
    ];
    assert!(!cli(dir.path(), &args).status.success());
    let receipt = success(cli(dir.path(), &[&["--experimental"][..], &args].concat()));
    assert!(receipt.contains(&format!("foreign={commit}")));
    let origins = success(cli(dir.path(), &["git", "origins"]));
    assert!(origins.contains("foreign-parents=1 tracking=none ancestry=foreign"));
    assert!(success(cli(dir.path(), &["status"])).contains("Git ancestry: not inspected"));
    let bytes = dir.path().join("supplier");
    fs::write(&bytes, b"native CLI\r\n").unwrap();
    success(cli(
        dir.path(),
        &[
            "--experimental",
            "source",
            "put",
            "cli",
            "--file",
            bytes.to_str().unwrap(),
            "--expected-generation",
            "1",
            "--request-key",
            "cli-edit",
        ],
    ));
    assert!(
        !cli(
            dir.path(),
            &["--experimental", "git", "export", "--git-dir", foreign]
        )
        .status
        .success()
    );
    success(cli(
        dir.path(),
        &[
            "--experimental",
            "commit",
            "--author",
            "CLI <cli@example.test>",
            "-m",
            "cli child",
            "--expected-generation",
            "2",
            "--request-key",
            "cli-child",
        ],
    ));
    assert!(
        !cli(dir.path(), &["git", "export", "--git-dir", foreign])
            .status
            .success()
    );
    let out = success(cli(
        dir.path(),
        &["--experimental", "git", "export", "--git-dir", foreign],
    ));
    assert!(out.contains("policy=avc.git.export/v1 publication=objects-only"));
    let oid = out
        .split_whitespace()
        .find_map(|s| s.strip_prefix("commit=sha1:"))
        .unwrap();
    assert_eq!(
        git(&f.git, &["show", &format!("{oid}:cli")], b""),
        b"native CLI\r\n"
    );
    assert_eq!(
        text(git(&f.git, &["rev-parse", &format!("{oid}^")], b"")),
        commit.replace("sha1:", "")
    );
    fs::rename(&f.git, f.git.with_extension("gone")).unwrap();
    assert_eq!(
        success(cli(dir.path(), &[&["--experimental"][..], &args].concat())),
        receipt
    );
}
