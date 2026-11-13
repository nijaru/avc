//! Faults at the SQLite boundary, not a simulation of OS power loss.
use super::*;
use crate::engine::Edit;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};

fn source(seed: u8, extra_files: usize) -> (Engine, Id) {
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let blob = engine.blob(vec![seed; 8192]).unwrap();
    let mut edits = vec![Edit {
        path: vec![b"payload".to_vec()],
        expected: Entry::Absent,
        value: Entry::File {
            blob,
            executable: false,
        },
    }];
    for i in 0..extra_files {
        let blob = engine
            .blob(format!("distinct small payload {seed} {i}").into_bytes())
            .unwrap();
        edits.push(Edit {
            path: vec![format!("item-{i:06}").into_bytes()],
            expected: Entry::Absent,
            value: Entry::File {
                blob,
                executable: false,
            },
        });
    }
    let root = engine.edit(empty, &edits).unwrap();
    (engine, root)
}

fn assert_restorable(repo: &Repository, root: Id, seed: u8) {
    let engine = repo.load_root(root).unwrap();
    let Entry::File { blob, .. } = engine.read(root, &[b"payload".to_vec()]).unwrap() else {
        panic!("missing file")
    };
    assert_eq!(
        engine.object(blob).unwrap(),
        &Object::Blob(vec![seed; 8192])
    );
    let mut stream = Vec::new();
    repo.export(root, &mut stream).unwrap();
    let project = tempfile::tempdir().unwrap();
    let mut independent = Repository::init(project.path()).unwrap();
    assert_eq!(
        independent
            .import(stream.as_slice(), "restore")
            .unwrap()
            .after
            .root,
        root
    );
    let (selected, restored) = independent.load().unwrap();
    assert_eq!(selected.root, root);
    assert_eq!(
        restored.object(blob).unwrap(),
        &Object::Blob(vec![seed; 8192])
    );
}

#[test]
fn failed_commit_and_failed_rollback_cannot_acknowledge_own_uncommitted_receipt() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let (old, old_root) = source(17, 0);
    let before = repo
        .commit_source(&old, old_root, 0, "before")
        .unwrap()
        .after;
    let (next, next_root) = source(29, 0);
    let observer = Repository::open(project.path()).unwrap();
    // COMMIT fails before execution and Transaction::drop cannot prepare its
    // ROLLBACK either. This models an explicitly surviving transaction; ordinary
    // SQLITE_IOERR does not necessarily leave one active.
    repo.connection
        .authorizer(Some(|ctx: AuthContext<'_>| {
            if matches!(
                ctx.action,
                AuthAction::Transaction { operation } if operation != TransactionOperation::Begin
            ) {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }))
        .unwrap();
    assert!(
        repo.commit_source(&next, next_root, before.generation, "unfinished")
            .is_err()
    );
    repo.connection
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .unwrap();
    assert_eq!(observer.status().unwrap(), before);
    let replay = repo.commit_source(&Engine::new(), next_root, before.generation, "unfinished");
    let receipt = repo.receipt("unfinished");
    let selected = repo.status();
    assert!(
        matches!(replay, Err(Error::RecoveryRequired))
            && matches!(receipt, Err(Error::RecoveryRequired))
            && matches!(selected, Err(Error::RecoveryRequired)),
        "unfinished handle exposed an uncommitted outcome: replay={replay:?} receipt={receipt:?} selected={selected:?}"
    );
    assert!(matches!(
        repo.load_root(next_root),
        Err(Error::RecoveryRequired)
    ));
    assert!(matches!(
        repo.preparation_base(before.generation, "unfinished"),
        Err(Error::RecoveryRequired)
    ));
    let mut output = Vec::new();
    assert!(matches!(
        repo.export(next_root, &mut output),
        Err(Error::RecoveryRequired)
    ));
    assert!(output.is_empty());
    let output_path = project.path().join("must-not-create.avc");
    assert!(matches!(
        repo.export_file(next_root, &output_path),
        Err(Error::RecoveryRequired)
    ));
    assert!(!output_path.exists());
    assert!(matches!(
        repo.import(std::io::empty(), "must-not-read"),
        Err(Error::RecoveryRequired)
    ));
    assert!(matches!(
        repo.commit(
            "Author",
            "must not write",
            before.generation,
            "must-not-author"
        ),
        Err(Error::RecoveryRequired)
    ));
    assert!(matches!(
        repo.revision(next_root),
        Err(Error::RecoveryRequired)
    ));
    assert!(matches!(repo.log(None, 20), Err(Error::RecoveryRequired)));
    assert!(matches!(
        repo.amend("", "", before.generation, "must-not-amend"),
        Err(Error::RecoveryRequired)
    ));
    assert!(matches!(
        repo.undo("", before.generation, "must-not-undo"),
        Err(Error::RecoveryRequired)
    ));
    assert!(matches!(
        repo.rebase(next_root, before.generation, "must-not-rebase"),
        Err(Error::RecoveryRequired)
    ));
    let foreign = "0000000000000000000000000000000000000001".parse().unwrap();
    assert!(matches!(
        repo.import_git(project.path(), foreign, "must-not-git-import"),
        Err(Error::RecoveryRequired)
    ));
    assert!(matches!(
        repo.export_git(project.path(), next_root),
        Err(Error::RecoveryRequired)
    ));
    assert!(matches!(
        repo.git_origin(next_root),
        Err(Error::RecoveryRequired)
    ));
    assert!(matches!(
        repo.git_origins(next_root),
        Err(Error::RecoveryRequired)
    ));
    assert!(matches!(
        repo.export_history(&mut output),
        Err(Error::RecoveryRequired)
    ));
    assert!(output.is_empty());
    assert!(matches!(
        repo.export_history_file(&output_path),
        Err(Error::RecoveryRequired)
    ));
    assert!(!output_path.exists());
    assert!(matches!(
        repo.import_history(std::io::empty(), "must-not-import"),
        Err(Error::RecoveryRequired)
    ));
    assert_restorable(&observer, before.root, 17);
    drop(repo);
    drop(observer);
    let mut reopened = Repository::open(project.path()).unwrap();
    assert!(reopened.receipt("unfinished").unwrap().is_none());
    assert_eq!(
        reopened
            .commit_source(&next, next_root, before.generation, "unfinished")
            .unwrap()
            .after
            .generation,
        before.generation + 1
    );
    assert_restorable(&reopened, next_root, 29);
}

#[cfg(unix)]
#[test]
fn posix_lock_probe_child() {
    let Some(path) = std::env::var_os("AVC_TEST_LOCK_DATABASE") else {
        return;
    };
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    match rustix::fs::fcntl_lock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => std::process::exit(87),
        Err(rustix::io::Errno::AGAIN | rustix::io::Errno::ACCESS) => std::process::exit(86),
        Err(error) => panic!("unexpected lock error: {error}"),
    }
}

#[cfg(unix)]
#[test]
fn repository_open_preserves_sqlites_posix_database_locks() {
    fn assert_locked(path: &Path) {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "repository::fault_tests::posix_lock_probe_child",
                "--nocapture",
            ])
            .env("AVC_TEST_LOCK_DATABASE", path)
            .status()
            .unwrap();
        assert_eq!(
            status.code(),
            Some(86),
            "a live WAL connection lost its main-database lock"
        );
    }
    let project = tempfile::tempdir().unwrap();
    let repo = Repository::init(project.path()).unwrap();
    let database = project.path().join(".avc/store.sqlite");
    assert_locked(&database);
    drop(repo);
    let repo = Repository::open(project.path()).unwrap();
    assert_locked(&database);
    drop(repo);
}

#[test]
fn incomplete_checkpoint_cannot_acknowledge_a_receipt() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let (old, old_root) = source(17, 0);
    let before = repo
        .commit_source(&old, old_root, 0, "before")
        .unwrap()
        .after;
    let reader = Connection::open(project.path().join(".avc/store.sqlite")).unwrap();
    reader.execute_batch("BEGIN").unwrap();
    let held: Vec<u8> = reader
        .query_row("SELECT root FROM selection", [], |r| r.get(0))
        .unwrap();
    assert_eq!(Id::from_bytes(&held).unwrap(), before.root);
    let (next, next_root) = source(29, 0);
    repo.commit_source(&next, next_root, before.generation, "held")
        .unwrap();
    repo.connection
        .busy_timeout(Duration::from_millis(5))
        .unwrap();
    assert!(
        repo.receipt("held").is_err(),
        "a partial FULL checkpoint is not a completed durability fence"
    );
    reader.execute_batch("ROLLBACK").unwrap();
    let Receipt::Selection(held) = repo.receipt("held").unwrap().unwrap() else {
        panic!("expected selection receipt");
    };
    assert_eq!(held.after.root, next_root);
    assert_restorable(&repo, before.root, 17);
    assert_restorable(&repo, next_root, 29);
}

#[test]
fn sqlite_page_quota_rolls_back_without_losing_departed_source() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let (old, old_root) = source(17, 0);
    let before = repo
        .commit_source(&old, old_root, 0, "before")
        .unwrap()
        .after;
    let pages: i64 = repo
        .connection
        .pragma_query_value(None, "page_count", |r| r.get(0))
        .unwrap();
    repo.connection
        .pragma_update(None, "max_page_count", pages)
        .unwrap();
    let (next, next_root) = source(29, 1000);
    let error = repo
        .commit_source(&next, next_root, before.generation, "quota")
        .unwrap_err();
    assert!(
        matches!(error, Error::Sqlite(rusqlite::Error::SqliteFailure(ref e, _)) if e.code == rusqlite::ErrorCode::DiskFull),
        "{error:?}"
    );
    assert_eq!(repo.status().unwrap(), before);
    assert!(repo.receipt("quota").unwrap().is_none());
    assert_restorable(&repo, before.root, 17);
    repo.connection
        .pragma_update(None, "max_page_count", 1_000_000)
        .unwrap();
    let outcome = repo
        .commit_source(&next, next_root, before.generation, "quota")
        .unwrap();
    assert_eq!(outcome.after.generation, before.generation + 1);
    assert_restorable(&repo, before.root, 17);
    assert_restorable(&repo, next_root, 29);
}

#[test]
fn denied_sqlite_mutation_boundaries_leave_no_partial_authority() {
    for boundary in ["begin", "objects", "selection", "operations", "commit"] {
        let project = tempfile::tempdir().unwrap();
        let mut repo = Repository::init(project.path()).unwrap();
        let (old, old_root) = source(17, 0);
        let before = repo
            .commit_source(&old, old_root, 0, "before")
            .unwrap()
            .after;
        let (next, next_root) = source(29, 0);
        if boundary == "commit" {
            repo.connection.commit_hook(Some(|| true)).unwrap();
        } else {
            repo.connection
                .authorizer(Some(move |ctx: AuthContext<'_>| {
                    let deny = match ctx.action {
                        AuthAction::Transaction {
                            operation: TransactionOperation::Begin,
                        } => boundary == "begin",
                        AuthAction::Insert { table_name } => table_name == boundary,
                        AuthAction::Update { table_name, .. } => table_name == boundary,
                        _ => false,
                    };
                    if deny {
                        Authorization::Deny
                    } else {
                        Authorization::Allow
                    }
                }))
                .unwrap();
        }
        assert!(
            repo.commit_source(&next, next_root, before.generation, "fault")
                .is_err(),
            "{boundary}"
        );
        repo.connection
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .unwrap();
        repo.connection.commit_hook(None::<fn() -> bool>).unwrap();
        assert_eq!(repo.status().unwrap(), before, "{boundary}");
        assert!(repo.receipt("fault").unwrap().is_none(), "{boundary}");
        assert_restorable(&repo, before.root, 17);
        drop(repo);
        let mut reopened = Repository::open(project.path()).unwrap();
        assert_eq!(reopened.status().unwrap(), before);
        assert!(reopened.receipt("fault").unwrap().is_none());
        assert_eq!(
            reopened
                .commit_source(&next, next_root, before.generation, "fault")
                .unwrap()
                .after
                .generation,
            before.generation + 1
        );
        assert_restorable(&reopened, before.root, 17);
        assert_restorable(&reopened, next_root, 29);
    }
}

#[test]
fn post_commit_wal_hook_error_reconciles_the_already_committed_outcome() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let (old, old_root) = source(17, 0);
    let before = repo
        .commit_source(&old, old_root, 0, "before")
        .unwrap()
        .after;
    let (next, next_root) = source(29, 0);
    repo.connection.wal_hook(Some(|_, _| {
        Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_IOERR),
            Some("post-commit hook failure".into()),
        ))
    }));
    assert!(
        repo.commit_source(&next, next_root, before.generation, "ambiguous")
            .is_err()
    );
    repo.connection.wal_hook(None);
    let Receipt::Selection(outcome) = repo.receipt("ambiguous").unwrap().unwrap() else {
        panic!("expected selection receipt");
    };
    assert_eq!(outcome.before, before);
    assert_eq!(outcome.after.root, next_root);
    assert_eq!(
        repo.commit_source(&Engine::new(), next_root, before.generation, "ambiguous")
            .unwrap(),
        outcome
    );
    assert_restorable(&repo, before.root, 17);
    assert_restorable(&repo, next_root, 29);
    drop(repo);
    let reopened = Repository::open(project.path()).unwrap();
    assert_eq!(
        reopened.receipt("ambiguous").unwrap().unwrap(),
        Receipt::Selection(outcome)
    );
}
