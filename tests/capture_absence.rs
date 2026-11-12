#![cfg(unix)]
use avc::{
    Entry, Id, Object,
    capture::{CaptureMode, report},
    engine::Edit,
    repository::{CaptureScope, Error, Operation, Repository},
};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn path(name: &str) -> Vec<Vec<u8>> {
    if name.is_empty() {
        Vec::new()
    } else {
        name.split('/').map(|p| p.as_bytes().to_vec()).collect()
    }
}
fn cli(project: &Path, args: &[&str]) -> Output {
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
fn entry(repo: &Repository, root: Id, name: &str) -> Entry {
    repo.load_root(root)
        .unwrap()
        .read(root, &path(name))
        .unwrap()
}
fn bytes(repo: &Repository, root: Id, name: &str) -> Vec<u8> {
    let engine = repo.load_root(root).unwrap();
    let Entry::File { blob, .. } = engine.read(root, &path(name)).unwrap() else {
        panic!("not a file")
    };
    let Object::Blob(data) = engine.object(blob).unwrap() else {
        panic!("not a blob")
    };
    data.clone()
}

#[test]
fn actual_cli_prunes_only_proven_missing_bindings_and_commit_does_not_rescan() {
    let project = tempfile::tempdir().unwrap();
    let input = tempfile::tempdir().unwrap();
    fs::create_dir_all(input.path().join("src/tree")).unwrap();
    fs::create_dir(input.path().join("src/empty")).unwrap();
    for name in ["src/kept", "src/gone", "src/tree/deep", "outside"] {
        fs::write(input.path().join(name), name.as_bytes()).unwrap();
    }
    let mut repo = Repository::init(project.path()).unwrap();
    repo.capture(
        &CaptureScope::recursive(input.path(), vec![path("src"), path("outside")]).unwrap(),
        0,
        "initial",
    )
    .unwrap();
    let authored = repo
        .commit("A <a@example.com>", "before", 1, "author")
        .unwrap();
    fs::remove_file(input.path().join("src/gone")).unwrap();
    fs::remove_dir_all(input.path().join("src/tree")).unwrap();
    fs::remove_dir(input.path().join("src/empty")).unwrap();
    fs::write(input.path().join("src/kept"), b"new selected bytes").unwrap();
    fs::write(input.path().join("outside"), b"unselected live change").unwrap();
    let from = input.path().to_str().unwrap();
    let refused = cli(
        project.path(),
        &[
            "--experimental",
            "capture",
            "--from",
            from,
            "--recursive",
            "--expected-generation",
            "2",
            "--request-key",
            "default",
            "src",
        ],
    );
    assert!(!refused.status.success());
    assert_eq!(repo.status().unwrap(), authored.after);
    assert!(repo.receipt("default").unwrap().is_none());
    let output = success(
        project.path(),
        &[
            "--experimental",
            "capture",
            "--from",
            from,
            "--recursive",
            "--remove-missing",
            "--expected-generation",
            "2",
            "--request-key",
            "remove",
            "src",
        ],
    );
    for name in ["src/empty", "src/gone", "src/tree"] {
        assert!(
            output.contains(&format!("Removed from selected source: {name}")),
            "{output}"
        );
    }
    let receipt = repo.receipt("remove").unwrap().unwrap();
    assert_eq!(receipt.after.generation, 3);
    assert_eq!(receipt.after.revision, authored.after.revision);
    for name in ["src/empty", "src/gone", "src/tree"] {
        assert_eq!(entry(&repo, receipt.after.root, name), Entry::Absent);
    }
    assert_eq!(bytes(&repo, receipt.after.root, "outside"), b"outside");
    assert_eq!(
        bytes(&repo, receipt.after.root, "src/kept"),
        b"new selected bytes"
    );
    assert_eq!(bytes(&repo, receipt.before.root, "src/gone"), b"src/gone");
    fs::write(input.path().join("src/gone"), b"recreated after capture").unwrap();
    success(
        project.path(),
        &[
            "--experimental",
            "commit",
            "--author",
            "A <a@example.com>",
            "--message",
            "pinned absence",
            "--expected-generation",
            "3",
            "--request-key",
            "commit",
        ],
    );
    let after = repo.status().unwrap();
    assert_eq!(
        repo.revision(after.revision.unwrap()).unwrap().root,
        receipt.after.root
    );
    assert_eq!(
        fs::read(input.path().join("src/gone")).unwrap(),
        b"recreated after capture"
    );
}

#[test]
fn removal_policy_is_retry_identity_even_without_removals_and_replays_before_filesystem_access() {
    let project = tempfile::tempdir().unwrap();
    let input = tempfile::tempdir().unwrap();
    fs::write(input.path().join("a"), b"selected").unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let ordinary = CaptureScope::recursive(input.path(), vec![Vec::new()]).unwrap();
    let removing =
        CaptureScope::recursive_removing_missing(input.path(), vec![Vec::new()]).unwrap();
    let initial = repo.capture(&ordinary, 0, "initial").unwrap();
    let captured = repo.capture(&removing, 1, "policy").unwrap();
    assert_eq!(captured.before.root, captured.after.root);
    assert_eq!(captured.after.root, initial.after.root);
    let evidence = repo.capture_observation("policy").unwrap().unwrap();
    assert!(report(&evidence).unwrap().removed.is_empty());
    repo.commit("A <a@example.com>", "later", 2, "author")
        .unwrap();
    fs::remove_dir_all(input.path()).unwrap();
    let later = repo.status().unwrap();
    for different in [
        ordinary,
        CaptureScope::new(input.path(), vec![path("a")]).unwrap(),
    ] {
        assert!(matches!(
            repo.capture(&different, 1, "policy"),
            Err(Error::RequestKeyReuse(_))
        ));
    }
    assert!(matches!(
        repo.capture(&removing, 2, "policy"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert_eq!(repo.capture(&removing, 1, "policy").unwrap(), captured);
    assert_eq!(repo.status().unwrap(), later);
    drop(repo);
    let mut repo = Repository::open(project.path()).unwrap();
    assert_eq!(repo.capture(&removing, 1, "policy").unwrap(), captured);
    assert_eq!(
        repo.capture_observation("policy").unwrap().unwrap(),
        evidence
    );
    let Operation::Capture { scope } = captured.operation else {
        panic!("not capture")
    };
    assert_eq!(scope.mode(), CaptureMode::RecursiveRemovingMissing);
}

#[test]
fn cancelling_pruning_restores_retained_source_and_head_not_deleted_live_files() {
    let project = tempfile::tempdir().unwrap();
    let input = tempfile::tempdir().unwrap();
    fs::create_dir(input.path().join("src")).unwrap();
    fs::write(input.path().join("src/a"), b"retained").unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    repo.capture(
        &CaptureScope::recursive(input.path(), vec![path("src")]).unwrap(),
        0,
        "initial",
    )
    .unwrap();
    let before = repo
        .commit("A <a@example.com>", "before", 1, "author")
        .unwrap();
    fs::remove_file(input.path().join("src/a")).unwrap();
    let captured = repo
        .capture(
            &CaptureScope::recursive_removing_missing(input.path(), vec![path("src")]).unwrap(),
            2,
            "remove",
        )
        .unwrap();
    assert_eq!(entry(&repo, captured.after.root, "src/a"), Entry::Absent);
    assert_eq!(
        report(&repo.capture_observation("remove").unwrap().unwrap())
            .unwrap()
            .removed,
        vec![path("src/a")]
    );
    let restored = repo.undo("remove", 3, "cancel").unwrap();
    assert_eq!(restored.after.root, before.after.root);
    assert_eq!(restored.after.revision, before.after.revision);
    assert_eq!(restored.after.generation, 4);
    assert_eq!(bytes(&repo, restored.after.root, "src/a"), b"retained");
    assert!(!input.path().join("src/a").exists());
    assert_eq!(repo.log(None, 10).unwrap().len(), 1);
    assert_eq!(
        repo.capture(
            &CaptureScope::recursive_removing_missing(input.path(), vec![path("src")]).unwrap(),
            2,
            "remove"
        )
        .unwrap(),
        captured
    );
    assert_eq!(repo.status().unwrap(), restored.after);
}

#[test]
fn failed_live_reads_and_protected_subtree_pruning_publish_no_partial_candidate() {
    let project = tempfile::tempdir().unwrap();
    let input = tempfile::tempdir().unwrap();
    fs::create_dir_all(input.path().join("src/branch")).unwrap();
    fs::write(input.path().join("src/branch/a"), b"a").unwrap();
    fs::write(input.path().join("src/changed"), b"before").unwrap();
    fs::write(input.path().join("src/z"), b"before").unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let initial = repo
        .capture(
            &CaptureScope::recursive(input.path(), vec![path("src")]).unwrap(),
            0,
            "initial",
        )
        .unwrap();
    let mut engine = repo.load_root(initial.after.root).unwrap();
    let blob = engine
        .blob(b"protected administrative source data".to_vec())
        .unwrap();
    let protected = engine
        .edit(
            initial.after.root,
            &[Edit {
                path: path("src/branch/.git"),
                expected: Entry::Absent,
                value: Entry::File {
                    blob,
                    executable: false,
                },
            }],
        )
        .unwrap();
    repo.commit_source(&engine, protected, 1, "protect")
        .unwrap();
    let before = repo.status().unwrap();
    let db = rusqlite::Connection::open(project.path().join(".avc/store.sqlite")).unwrap();
    let counts = || {
        db.query_row("SELECT (SELECT count(*) FROM objects), (SELECT count(*) FROM operations), (SELECT count(*) FROM capture_observations)", [], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))).unwrap()
    };
    let original_counts = counts();
    fs::remove_dir_all(input.path().join("src/branch")).unwrap();
    fs::write(input.path().join("src/changed"), b"must not be admitted").unwrap();
    let requested =
        CaptureScope::recursive_removing_missing(input.path(), vec![path("src")]).unwrap();
    assert!(repo.capture(&requested, 2, "protected").is_err());
    assert_eq!(repo.status().unwrap(), before);
    assert_eq!(counts(), original_counts);
    assert!(repo.receipt("protected").unwrap().is_none());
    assert!(repo.capture_observation("protected").unwrap().is_none());
    fs::create_dir(input.path().join("src/branch")).unwrap();
    fs::write(input.path().join("src/branch/a"), b"a").unwrap();
    fs::remove_file(input.path().join("src/z")).unwrap();
    assert!(
        Command::new("mkfifo")
            .arg(input.path().join("src/z"))
            .status()
            .unwrap()
            .success()
    );
    assert!(repo.capture(&requested, 2, "unreadable").is_err());
    assert_eq!(repo.status().unwrap(), before);
    assert_eq!(counts(), original_counts);
    assert!(repo.receipt("unreadable").unwrap().is_none());
    assert!(repo.capture_observation("unreadable").unwrap().is_none());
}

#[test]
fn removal_requires_recursive_consent_and_refuses_missing_parents_untracked_scopes_and_type_replacement()
 {
    let project = tempfile::tempdir().unwrap();
    let input = tempfile::tempdir().unwrap();
    fs::create_dir_all(input.path().join("src/branch")).unwrap();
    fs::write(input.path().join("src/branch/a"), b"a").unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let initial = repo
        .capture(
            &CaptureScope::recursive(input.path(), vec![path("src")]).unwrap(),
            0,
            "initial",
        )
        .unwrap();
    let output = cli(
        project.path(),
        &[
            "--experimental",
            "capture",
            "--remove-missing",
            "--expected-generation",
            "1",
            "--request-key",
            "implicit",
            "src",
        ],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--recursive"));
    assert!(repo.receipt("implicit").unwrap().is_none());
    fs::remove_dir_all(input.path().join("src/branch")).unwrap();
    for (key, requested) in [("parent", "src/branch/a"), ("new", "src/never-selected")] {
        let scope =
            CaptureScope::recursive_removing_missing(input.path(), vec![path(requested)]).unwrap();
        assert!(repo.capture(&scope, 1, key).is_err());
        assert!(repo.receipt(key).unwrap().is_none());
        assert_eq!(repo.status().unwrap(), initial.after);
    }
    fs::write(
        input.path().join("src/branch"),
        b"not subtree removal consent",
    )
    .unwrap();
    let scope = CaptureScope::recursive_removing_missing(input.path(), vec![path("src")]).unwrap();
    assert!(repo.capture(&scope, 1, "replacement").is_err());
    assert!(repo.receipt("replacement").unwrap().is_none());
    assert_eq!(repo.status().unwrap(), initial.after);
}
