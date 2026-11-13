#![cfg(unix)]
use avc::{
    Entry, Object,
    repository::{CaptureScope, Error, Operation, Repository},
};
use rusqlite::Connection;
use std::{
    fs,
    path::Path,
    process::Command,
    sync::{Arc, Barrier},
};
fn paths(names: &[&str]) -> Vec<Vec<Vec<u8>>> {
    names
        .iter()
        .map(|s| s.split('/').map(|s| s.as_bytes().to_vec()).collect())
        .collect()
}
fn scope(dir: &Path, names: &[&str]) -> CaptureScope {
    CaptureScope::new(dir, paths(names)).unwrap()
}
fn bytes(repo: &Repository, root: avc::Id, name: &str) -> Vec<u8> {
    let engine = repo.load_root(root).unwrap();
    let Entry::File { blob, .. } = engine.read(root, &paths(&[name])[0]).unwrap() else {
        panic!("not file")
    };
    let Object::Blob(bytes) = engine.object(blob).unwrap() else {
        panic!("not blob")
    };
    bytes.clone()
}
fn database(dir: &Path) -> Connection {
    Connection::open(dir.join(".avc/store.sqlite")).unwrap()
}
#[test]
fn capture_pins_source_and_scope_retry_never_rereads_files_or_changes_later_selection() {
    let project = tempfile::tempdir().unwrap();
    let input = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    fs::write(input.path().join("a"), b"preview\0\r\n").unwrap();
    fs::write(input.path().join("b"), b"unselected").unwrap();
    let scope = scope(input.path(), &["a"]);
    let receipt = repo.capture(&scope, 0, "observe").unwrap();
    assert_eq!(receipt.after.revision, None);
    assert_eq!(
        receipt.operation,
        Operation::Capture {
            scope: scope.clone()
        }
    );
    assert_eq!(bytes(&repo, receipt.after.root, "a"), b"preview\0\r\n");
    assert!(
        !repo
            .capture_observation("observe")
            .unwrap()
            .unwrap()
            .is_empty()
    );
    fs::write(input.path().join("a"), b"later live edit").unwrap();
    let commit = repo
        .commit("A <a@example.com>", "pinned", 1, "author")
        .unwrap();
    assert_eq!(
        repo.revision(commit.after.revision.unwrap()).unwrap().root,
        receipt.after.root
    );
    assert_eq!(
        fs::read(input.path().join("a")).unwrap(),
        b"later live edit"
    );
    fs::remove_dir_all(input.path()).unwrap();
    let later = repo.status().unwrap();
    assert_eq!(repo.capture(&scope, 0, "observe").unwrap(), receipt);
    assert_eq!(repo.status().unwrap(), later);
    assert!(matches!(
        repo.capture(&scope, 1, "observe"),
        Err(Error::RequestKeyReuse(_))
    ));
    let different = self::scope(input.path(), &["b"]);
    assert!(matches!(
        repo.capture(&different, 0, "observe"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert!(matches!(
        repo.capture(&scope, 0, "author"),
        Err(Error::RequestKeyReuse(_))
    ));
    drop(repo);
    let mut repo = Repository::open(project.path()).unwrap();
    assert_eq!(repo.capture(&scope, 0, "observe").unwrap(), receipt);
}
#[test]
fn recapture_preserves_authored_head_and_unselected_native_bytes() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    fs::write(project.path().join("a"), b"a1").unwrap();
    fs::write(project.path().join("b"), b"b1").unwrap();
    repo.capture(&scope(project.path(), &["a", "b"]), 0, "both")
        .unwrap();
    let authored = repo
        .commit("A <a@example.com>", "initial", 1, "author")
        .unwrap();
    fs::write(project.path().join("a"), b"a2").unwrap();
    fs::write(project.path().join("b"), b"b2 live only").unwrap();
    let capture = repo
        .capture(&scope(project.path(), &["a"]), 2, "one")
        .unwrap();
    assert_eq!(capture.after.revision, authored.after.revision);
    assert_eq!(bytes(&repo, capture.after.root, "a"), b"a2");
    assert_eq!(bytes(&repo, capture.after.root, "b"), b"b1");
    assert_eq!(fs::read(project.path().join("b")).unwrap(), b"b2 live only");
    let commit = repo
        .commit("A <a@example.com>", "one selected file", 3, "next")
        .unwrap();
    assert_eq!(
        repo.revision(commit.after.revision.unwrap())
            .unwrap()
            .parents,
        vec![authored.after.revision.unwrap()]
    );
}
#[test]
fn cancel_capture_restores_source_not_live_files_and_authors_nothing() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    fs::write(project.path().join("a"), b"captured").unwrap();
    fs::write(project.path().join("b"), b"not selected").unwrap();
    let before = repo.status().unwrap();
    let capture = repo
        .capture(&scope(project.path(), &["a"]), 0, "capture")
        .unwrap();
    fs::write(project.path().join("a"), b"later").unwrap();
    let undo = repo.undo("capture", 1, "cancel").unwrap();
    assert_eq!(undo.after.root, before.root);
    assert_eq!(undo.after.revision, None);
    assert!(repo.log(None, 10).unwrap().is_empty());
    assert_eq!(fs::read(project.path().join("a")).unwrap(), b"later");
    assert_eq!(fs::read(project.path().join("b")).unwrap(), b"not selected");
    assert_eq!(bytes(&repo, capture.after.root, "a"), b"captured");
    assert!(repo.capture_observation("capture").unwrap().is_some());
}
#[test]
fn failed_reads_and_rejected_scopes_admit_nothing_and_do_not_infer_deletion() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    fs::write(project.path().join("a"), b"existing").unwrap();
    repo.capture(&scope(project.path(), &["a"]), 0, "first")
        .unwrap();
    let before = repo.status().unwrap();
    let db = database(project.path());
    let counts = || {
        db.query_row("SELECT (SELECT count(*) FROM objects), (SELECT count(*) FROM operations), (SELECT count(*) FROM capture_observations)", [], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?))).unwrap()
    };
    let initial = counts();
    fs::remove_file(project.path().join("a")).unwrap();
    assert!(
        repo.capture(&scope(project.path(), &["a"]), 1, "missing")
            .is_err()
    );
    assert_eq!(repo.status().unwrap(), before);
    assert_eq!(counts(), initial);
    assert_eq!(bytes(&repo, before.root, "a"), b"existing");
    assert!(CaptureScope::new(project.path(), paths(&[".avc/store.sqlite"])).is_err());
    assert!(CaptureScope::new(project.path(), paths(&["x/../a"])).is_err());
    assert!(CaptureScope::new(project.path(), paths(&["a", "a"])).is_err());
    assert!(CaptureScope::new(project.path(), vec![]).is_err());
    assert!(
        repo.capture(&scope(project.path(), &["a"]), 0, "stale")
            .is_err()
    );
    assert_eq!(counts(), initial);
}
#[test]
fn receipt_failure_rolls_back_source_objects_and_witnesses_together() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    fs::write(project.path().join("a"), b"new object").unwrap();
    let before = repo.status().unwrap();
    let db = database(project.path());
    db.execute_batch("CREATE TRIGGER deny_capture BEFORE INSERT ON operations WHEN NEW.kind = 'capture' BEGIN SELECT RAISE(ABORT, 'deny capture'); END;").unwrap();
    assert!(
        repo.capture(&scope(project.path(), &["a"]), 0, "blocked")
            .is_err()
    );
    assert_eq!(repo.status().unwrap(), before);
    assert_eq!(
        db.query_row::<i64, _, _>("SELECT count(*) FROM objects", [], |r| r.get(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row::<i64, _, _>("SELECT count(*) FROM capture_observations", [], |r| r
            .get(0))
            .unwrap(),
        0
    );
    assert!(repo.receipt("blocked").unwrap().is_none());
    db.execute_batch("DROP TRIGGER deny_capture").unwrap();
    repo.capture(&scope(project.path(), &["a"]), 0, "blocked")
        .unwrap();
}
#[test]
fn witness_inspection_detects_damage_but_receipt_recovery_is_not_a_health_certificate() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    fs::write(project.path().join("a"), b"a").unwrap();
    let scope = scope(project.path(), &["a"]);
    let receipt = repo.capture(&scope, 0, "observe").unwrap();
    let db = database(project.path());
    assert!(
        db.execute("UPDATE capture_observations SET witnesses = X'00'", [])
            .is_err()
    );
    db.execute_batch("DROP TRIGGER capture_observations_no_update; UPDATE capture_observations SET witnesses = X'00';").unwrap();
    assert!(repo.capture_observation("observe").is_err());
    fs::remove_file(project.path().join("a")).unwrap();
    assert_eq!(repo.capture(&scope, 0, "observe").unwrap(), receipt);
}
#[test]
fn concurrent_same_capture_returns_one_original_observation() {
    let project = tempfile::tempdir().unwrap();
    drop(Repository::init(project.path()).unwrap());
    fs::write(project.path().join("a"), b"a").unwrap();
    let barrier = Arc::new(Barrier::new(2));
    // Opening performs its own FULL checkpoint and may legitimately refuse a
    // concurrent busy checkpoint. This race tests capture, not startup; finish
    // fallible setup before either worker can wait at the barrier.
    let repositories = [
        Repository::open(project.path()).unwrap(),
        Repository::open(project.path()).unwrap(),
    ];
    let workers: Vec<_> = repositories
        .into_iter()
        .map(|mut repo| {
            let scope = scope(project.path(), &["a"]);
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                repo.capture(&scope, 0, "same").unwrap()
            })
        })
        .collect();
    let results: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
    assert_eq!(results[0], results[1]);
    let db = database(project.path());
    assert_eq!(
        db.query_row::<i64, _, _>("SELECT count(*) FROM capture_observations", [], |r| r
            .get(0))
            .unwrap(),
        1
    );
}
#[test]
fn emitted_cancel_command_is_executable_and_does_not_author_or_touch_live_files() {
    let project = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_avc"))
            .arg("--repository")
            .arg(project.path())
            .args(args)
            .output()
            .unwrap()
    };
    assert!(run(&["--experimental", "init"]).status.success());
    fs::write(project.path().join("a"), b"selected").unwrap();
    let output = run(&[
        "--experimental",
        "capture",
        "--expected-generation",
        "0",
        "--request-key",
        "capture",
        "a",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::write(project.path().join("a"), b"later").unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let command = stdout
        .lines()
        .find_map(|line| line.strip_prefix("Cancel: avc "))
        .expect("actionable cancellation guidance");
    let args: Vec<_> = command
        .split_whitespace()
        .map(|s| match s {
            "CAPTURE_KEY" => "capture",
            "NEW_KEY" => "cancel",
            s => s,
        })
        .collect();
    let output = run(&args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let repo = Repository::open(project.path()).unwrap();
    assert_eq!(repo.status().unwrap().revision, None);
    assert_eq!(
        repo.load_root(repo.status().unwrap().root)
            .unwrap()
            .read(repo.status().unwrap().root, &paths(&["a"])[0])
            .unwrap(),
        Entry::Absent
    );
    assert_eq!(fs::read(project.path().join("a")).unwrap(), b"later");
}
#[test]
fn cli_reports_raw_directory_bytes_without_lossy_scope_substitution() {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
    let project = tempfile::tempdir().unwrap();
    let input = tempfile::tempdir().unwrap();
    let raw = input.path().join(OsStr::from_bytes(b"source-\xc3\xbf"));
    fs::create_dir(&raw).unwrap();
    fs::write(raw.join("a"), b"raw").unwrap();
    drop(Repository::init(project.path()).unwrap());
    let output = Command::new(env!("CARGO_BIN_EXE_avc"))
        .arg("--repository")
        .arg(project.path())
        .args(["--experimental", "capture", "--from"])
        .arg(&raw)
        .args(["--expected-generation", "0", "--request-key", "raw", "a"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("source-\\xc3\\xbf"));
    assert!(!text.contains('\u{fffd}'));
    let repo = Repository::open(project.path()).unwrap();
    let avc::repository::Receipt::Selection(receipt) = repo.receipt("raw").unwrap().unwrap() else {
        panic!("expected selection receipt");
    };
    let Operation::Capture { scope } = receipt.operation else {
        panic!("not capture");
    };
    assert_eq!(
        scope.directory().as_os_str().as_bytes(),
        std::path::absolute(raw).unwrap().as_os_str().as_bytes()
    );
}
#[test]
fn actual_cli_capture_review_commit_does_not_rescan_and_requires_opt_in() {
    let project = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_avc"))
            .arg("--repository")
            .arg(project.path())
            .args(args)
            .output()
            .unwrap()
    };
    assert!(run(&["--experimental", "init"]).status.success());
    fs::write(project.path().join("a"), b"preview").unwrap();
    assert!(
        !run(&[
            "capture",
            "--expected-generation",
            "0",
            "--request-key",
            "capture",
            "a"
        ])
        .status
        .success()
    );
    let capture = run(&[
        "--experimental",
        "capture",
        "--expected-generation",
        "0",
        "--request-key",
        "capture",
        "a",
    ]);
    assert!(
        capture.status.success(),
        "{}",
        String::from_utf8_lossy(&capture.stderr)
    );
    assert!(
        String::from_utf8(capture.stdout)
            .unwrap()
            .contains("Captured: a")
    );
    fs::write(project.path().join("a"), b"later").unwrap();
    assert!(
        run(&[
            "--experimental",
            "commit",
            "--author",
            "A <a@example.com>",
            "-m",
            "preview",
            "--expected-generation",
            "1",
            "--request-key",
            "commit"
        ])
        .status
        .success()
    );
    let repo = Repository::open(project.path()).unwrap();
    assert_eq!(bytes(&repo, repo.status().unwrap().root, "a"), b"preview");
    assert_eq!(fs::read(project.path().join("a")).unwrap(), b"later");
    assert!(run(&["receipt", "capture"]).status.success());
    assert!(run(&["capture", "--help"]).status.success());
}
