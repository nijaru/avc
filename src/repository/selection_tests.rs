use super::*;
use crate::engine::Edit;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};

fn fixture() -> (tempfile::TempDir, Repository, Id) {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let mut revisions = Vec::new();
    for (i, bytes) in [b"first".as_slice(), b"second".as_slice()]
        .into_iter()
        .enumerate()
    {
        let mut engine = Engine::new();
        let empty = engine.empty().unwrap();
        let blob = engine.blob(bytes.to_vec()).unwrap();
        let root = engine
            .edit(
                empty,
                &[Edit {
                    path: vec![b"file".to_vec()],
                    expected: Entry::Absent,
                    value: Entry::File {
                        blob,
                        executable: false,
                    },
                }],
            )
            .unwrap();
        repo.commit_source(
            &engine,
            root,
            repo.status().unwrap().generation,
            &format!("source-{i}"),
        )
        .unwrap();
        revisions.push(
            repo.commit(
                "Author",
                "revision",
                repo.status().unwrap().generation,
                &format!("author-{i}"),
            )
            .unwrap()
            .after
            .revision
            .unwrap(),
        );
    }
    (project, repo, revisions[0])
}

#[test]
fn selection_receipt_and_commit_failures_publish_both_source_and_head_or_neither() {
    for boundary in ["selection", "operations", "commit"] {
        let (project, mut repo, target) = fixture();
        let before = repo.status().unwrap();
        let observer = Repository::open(project.path()).unwrap();
        if boundary == "commit" {
            repo.connection.commit_hook(Some(|| true)).unwrap();
        } else {
            repo.connection
                .authorizer(Some(move |ctx: AuthContext<'_>| match ctx.action {
                    AuthAction::Update {
                        table_name: "selection",
                        ..
                    } if boundary == "selection" => Authorization::Deny,
                    AuthAction::Insert {
                        table_name: "operations",
                    } if boundary == "operations" => Authorization::Deny,
                    _ => Authorization::Allow,
                }))
                .unwrap();
        }
        assert!(
            repo.select_revision(target, before.generation, "select")
                .is_err()
        );
        repo.connection
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .unwrap();
        repo.connection.commit_hook(None::<fn() -> bool>).unwrap();
        assert_eq!(repo.status().unwrap(), before);
        assert_eq!(observer.status().unwrap(), before);
        assert!(repo.receipt("select").unwrap().is_none());
        let selected = repo
            .select_revision(target, before.generation, "select")
            .unwrap();
        assert_eq!(selected.after.revision, Some(target));
        assert_ne!(selected.after.root, before.root);
    }
}

#[test]
fn committed_but_failed_selection_recovers_without_current_or_target_reads() {
    let (_project, mut repo, target) = fixture();
    let before = repo.status().unwrap();
    repo.connection.wal_hook(Some(|_, _| {
        Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_IOERR),
            None,
        ))
    }));
    assert!(
        repo.select_revision(target, before.generation, "select")
            .is_err()
    );
    repo.connection.wal_hook(None);
    let Receipt::Selection(original) = repo.receipt("select").unwrap().unwrap() else {
        panic!("selection receipt")
    };
    assert_eq!(original.after.revision, Some(target));
    repo.connection
        .authorizer(Some(|ctx: AuthContext<'_>| match ctx.action {
            AuthAction::Read {
                table_name: "objects" | "selection" | "git_origins" | "git_exports",
                ..
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }))
        .unwrap();
    assert_eq!(
        repo.select_revision(target, before.generation, "select")
            .unwrap(),
        original
    );
    assert_eq!(
        repo.receipt("select").unwrap(),
        Some(Receipt::Selection(original))
    );
    repo.connection
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .unwrap();
}

#[test]
fn selection_replay_uses_checked_checkpoint_fence_after_releasing_transaction() {
    let (project, mut repo, target) = fixture();
    let before = repo.status().unwrap();
    let selected = repo
        .select_revision(target, before.generation, "select")
        .unwrap();
    let reader = Connection::open(project.path().join(".avc/store.sqlite")).unwrap();
    reader
        .execute_batch("BEGIN; SELECT count(*) FROM operations")
        .unwrap();
    repo.commit("Author", "later", selected.after.generation, "later")
        .unwrap();
    repo.connection.busy_timeout(Duration::ZERO).unwrap();
    assert!(matches!(
        repo.select_revision(target, before.generation, "select"),
        Err(Error::DurabilityPending)
    ));
    assert!(repo.connection.is_autocommit());
    reader.execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        repo.select_revision(target, before.generation, "select")
            .unwrap(),
        selected
    );
}

#[test]
fn select_sql_shape_requires_target_and_excludes_other_effects() {
    let (_project, repo, target) = fixture();
    let before = repo.status().unwrap();
    for (i, extra) in [
        "",
        ", ref_name",
        ", onto_revision",
        ", undo_target",
        ", capture_scope",
        ", import_origins",
        ", git_commit",
    ]
    .into_iter()
    .enumerate()
    {
        let value = match extra {
            "" => "NULL",
            ", ref_name" => "'name'",
            ", onto_revision" => "?2",
            ", undo_target" => "'author-0'",
            _ => "X'01'",
        };
        let (target_value, extra_value) = if extra.is_empty() {
            ("NULL", String::new())
        } else {
            ("?2", format!(", {value}"))
        };
        let sql = format!(
            "INSERT INTO operations (request_key, kind, before_root, before_generation, before_revision, after_root, after_generation, after_revision{extra}) VALUES (?1, 'select', ?3, 4, ?2, ?3, 5, {target_value}{extra_value})"
        );
        assert!(
            repo.connection
                .execute(
                    &sql,
                    params![
                        format!("bad-{i}"),
                        target.to_bytes().unwrap().as_slice(),
                        before.root.to_bytes().unwrap().as_slice()
                    ]
                )
                .is_err(),
            "accepted {extra}"
        );
    }
    assert_eq!(repo.status().unwrap(), before);
}
