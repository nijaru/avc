#![cfg(unix)]
use avc::{
    Entry, Id, Object,
    capture::report,
    repository::{CaptureScope, Operation, Repository},
};
use std::{fs, os::unix::fs::FileTypeExt, path::Path, process::Command};

fn path(name: &str) -> Vec<Vec<u8>> {
    name.split('/').map(|p| p.as_bytes().to_vec()).collect()
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
fn cli(project: &Path, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_avc"))
        .arg("--repository")
        .arg(project)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn actual_cli_direct_removals_do_not_capture_or_read_unselected_siblings() {
    let project = tempfile::tempdir().unwrap();
    let input = tempfile::tempdir().unwrap();
    fs::create_dir_all(input.path().join("src/branch")).unwrap();
    for name in ["src/branch/a", "src/gone", "src/keep"] {
        fs::write(input.path().join(name), name.as_bytes()).unwrap();
    }
    let mut repo = Repository::init(project.path()).unwrap();
    repo.capture(
        &CaptureScope::recursive(input.path(), vec![path("src")]).unwrap(),
        0,
        "initial",
    )
    .unwrap();
    let authored = repo
        .commit("A <a@example.com>", "before", 1, "author")
        .unwrap();
    fs::remove_file(input.path().join("src/gone")).unwrap();
    fs::remove_dir_all(input.path().join("src/branch")).unwrap();
    fs::write(input.path().join("src/keep"), b"unselected live edit").unwrap();
    fs::write(input.path().join("src/untracked"), b"not selected").unwrap();
    assert!(
        Command::new("mkfifo")
            .arg(input.path().join("src/pipe"))
            .status()
            .unwrap()
            .success()
    );
    let output = cli(
        project.path(),
        &[
            "--experimental",
            "capture",
            "--from",
            input.path().to_str().unwrap(),
            "--recursive",
            "--remove-missing",
            "--expected-generation",
            "2",
            "--request-key",
            "direct",
            "src/gone",
            "src/branch",
        ],
    );
    for name in ["src/branch", "src/gone"] {
        assert!(
            output.contains(&format!("Removed from selected source: {name}")),
            "{output}"
        );
    }
    let avc::repository::Receipt::Selection(captured) = repo.receipt("direct").unwrap().unwrap()
    else {
        panic!("expected selection receipt");
    };
    assert_eq!(captured.after.revision, authored.after.revision);
    assert_eq!(captured.after.generation, 3);
    for name in ["src/branch", "src/gone", "src/untracked", "src/pipe"] {
        assert_eq!(entry(&repo, captured.after.root, name), Entry::Absent);
    }
    assert_eq!(bytes(&repo, captured.after.root, "src/keep"), b"src/keep");
    assert_eq!(
        bytes(&repo, captured.before.root, "src/branch/a"),
        b"src/branch/a"
    );
    assert!(
        fs::symlink_metadata(input.path().join("src/pipe"))
            .unwrap()
            .file_type()
            .is_fifo()
    );
    assert_eq!(
        fs::read(input.path().join("src/keep")).unwrap(),
        b"unselected live edit"
    );
    let evidence = repo.capture_observation("direct").unwrap().unwrap();
    let coverage = report(&evidence).unwrap();
    assert_eq!(
        (coverage.files, coverage.symlinks, coverage.directories),
        (0, 0, 1)
    );
    assert_eq!(coverage.removed, vec![path("src/branch"), path("src/gone")]);
    assert!(coverage.excluded.is_empty());
    let Operation::Capture { scope } = &captured.operation else {
        panic!("not capture")
    };
    assert_eq!(scope.paths(), &[path("src/branch"), path("src/gone")]);
    fs::write(input.path().join("src/gone"), b"recreated after capture").unwrap();
    cli(
        project.path(),
        &[
            "--experimental",
            "commit",
            "--author",
            "A <a@example.com>",
            "--message",
            "selected absence only",
            "--expected-generation",
            "3",
            "--request-key",
            "commit",
        ],
    );
    assert_eq!(
        repo.revision(repo.status().unwrap().revision.unwrap())
            .unwrap()
            .root,
        captured.after.root
    );
    fs::remove_dir_all(input.path()).unwrap();
    drop(repo);
    let mut repo = Repository::open(project.path()).unwrap();
    assert_eq!(repo.capture(scope, 2, "direct").unwrap(), captured);
    assert_eq!(
        repo.capture_observation("direct").unwrap().unwrap(),
        evidence
    );
    assert_eq!(repo.status().unwrap().generation, 4);
}

#[test]
fn mixed_live_updates_and_direct_absences_publish_one_exact_candidate_or_nothing() {
    let project = tempfile::tempdir().unwrap();
    let input = tempfile::tempdir().unwrap();
    fs::create_dir(input.path().join("src")).unwrap();
    for name in ["a", "b", "c", "d"] {
        fs::write(input.path().join("src").join(name), name.as_bytes()).unwrap();
    }
    let mut repo = Repository::init(project.path()).unwrap();
    let initial = repo
        .capture(
            &CaptureScope::recursive(input.path(), vec![path("src")]).unwrap(),
            0,
            "initial",
        )
        .unwrap();
    fs::remove_file(input.path().join("src/a")).unwrap();
    fs::remove_file(input.path().join("src/c")).unwrap();
    fs::write(input.path().join("src/b"), b"selected new bytes").unwrap();
    fs::write(input.path().join("src/d"), b"unselected new bytes").unwrap();
    let scope = CaptureScope::recursive_removing_missing(
        input.path(),
        vec![path("src/a"), path("src/b"), path("src/c")],
    )
    .unwrap();
    let db = rusqlite::Connection::open(project.path().join(".avc/store.sqlite")).unwrap();
    let counts = || {
        db.query_row("SELECT (SELECT count(*) FROM objects), (SELECT count(*) FROM operations), (SELECT count(*) FROM capture_observations)", [], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))).unwrap()
    };
    let before_counts = counts();
    db.execute_batch("CREATE TRIGGER deny_selective_capture BEFORE INSERT ON operations WHEN NEW.request_key = 'mixed' BEGIN SELECT RAISE(ABORT, 'denied'); END;").unwrap();
    let refusal = repo.capture(&scope, 1, "mixed").unwrap_err().to_string();
    assert!(refusal.contains("denied"), "{refusal}");
    assert_eq!(repo.status().unwrap(), initial.after);
    assert_eq!(counts(), before_counts);
    assert!(repo.receipt("mixed").unwrap().is_none());
    assert!(repo.capture_observation("mixed").unwrap().is_none());
    db.execute_batch("DROP TRIGGER deny_selective_capture")
        .unwrap();
    let captured = repo.capture(&scope, 1, "mixed").unwrap();
    for name in ["src/a", "src/c"] {
        assert_eq!(entry(&repo, captured.after.root, name), Entry::Absent);
    }
    assert_eq!(
        bytes(&repo, captured.after.root, "src/b"),
        b"selected new bytes"
    );
    assert_eq!(bytes(&repo, captured.after.root, "src/d"), b"d");
    let coverage = report(&repo.capture_observation("mixed").unwrap().unwrap()).unwrap();
    assert_eq!(
        (coverage.files, coverage.symlinks, coverage.directories),
        (1, 0, 1)
    );
    assert_eq!(coverage.removed, vec![path("src/a"), path("src/c")]);
    let undone = repo.undo("mixed", 2, "cancel").unwrap();
    assert_eq!(undone.after.root, initial.after.root);
    assert!(!input.path().join("src/a").exists());
    assert_eq!(
        fs::read(input.path().join("src/b")).unwrap(),
        b"selected new bytes"
    );
}

#[test]
fn positive_capture_retry_cannot_turn_into_a_later_direct_removal() {
    let project = tempfile::tempdir().unwrap();
    let input = tempfile::tempdir().unwrap();
    fs::write(input.path().join("a"), b"original capture").unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let scope = CaptureScope::recursive_removing_missing(input.path(), vec![path("a")]).unwrap();
    let original = repo.capture(&scope, 0, "positive").unwrap();
    let evidence = repo.capture_observation("positive").unwrap().unwrap();
    fs::remove_file(input.path().join("a")).unwrap();
    assert_eq!(repo.capture(&scope, 0, "positive").unwrap(), original);
    assert_eq!(repo.status().unwrap(), original.after);
    assert_eq!(
        repo.capture_observation("positive").unwrap().unwrap(),
        evidence
    );
    let removed = repo.capture(&scope, 1, "negative").unwrap();
    assert_eq!(entry(&repo, removed.after.root, "a"), Entry::Absent);
    assert_eq!(bytes(&repo, original.after.root, "a"), b"original capture");
    assert_eq!(repo.capture(&scope, 0, "positive").unwrap(), original);
    assert_eq!(repo.status().unwrap(), removed.after);
    drop(repo);
    let mut repo = Repository::open(project.path()).unwrap();
    assert_eq!(repo.capture(&scope, 0, "positive").unwrap(), original);
    assert_eq!(repo.capture(&scope, 1, "negative").unwrap(), removed);
    assert_eq!(repo.status().unwrap().generation, 2);
}
