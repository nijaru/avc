use super::*;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};

fn fixture() -> (tempfile::TempDir, Repository, Id) {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let parent = repo
        .commit("Author", "first", 0, "first")
        .unwrap()
        .after
        .revision
        .unwrap();
    repo.commit("Author", "second", 1, "second").unwrap();
    (project, repo, parent)
}

fn objects(repo: &Repository) -> i64 {
    repo.connection
        .query_row("SELECT count(*) FROM objects", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn merge_revision_selection_receipt_and_commit_failures_publish_all_or_nothing() {
    for boundary in ["objects", "selection", "operations", "commit"] {
        let (project, mut repo, parent) = fixture();
        let before = repo.status().unwrap();
        let count = objects(&repo);
        let observer = Repository::open(project.path()).unwrap();
        if boundary == "commit" {
            repo.connection.commit_hook(Some(|| true)).unwrap();
        } else {
            repo.connection
                .authorizer(Some(move |ctx: AuthContext<'_>| match ctx.action {
                    AuthAction::Insert { table_name } if table_name == boundary => {
                        Authorization::Deny
                    }
                    AuthAction::Update {
                        table_name: "selection",
                        ..
                    } if boundary == "selection" => Authorization::Deny,
                    _ => Authorization::Allow,
                }))
                .unwrap();
        }
        assert!(
            repo.commit_merge(parent, "Author", "merge", 2, "merge")
                .is_err(),
            "{boundary}"
        );
        repo.connection
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .unwrap();
        repo.connection.commit_hook(None::<fn() -> bool>).unwrap();
        assert_eq!(repo.status().unwrap(), before);
        assert_eq!(observer.status().unwrap(), before);
        assert_eq!(objects(&repo), count);
        assert!(repo.receipt("merge").unwrap().is_none());
        let merged = repo
            .commit_merge(parent, "Author", "merge", 2, "merge")
            .unwrap();
        assert_eq!(merged.before, before);
        assert_eq!(merged.after.root, before.root);
        assert_eq!(objects(&repo), count + 1);
        assert_eq!(
            repo.revision(merged.after.revision.unwrap())
                .unwrap()
                .parents,
            [before.revision.unwrap(), parent]
        );
    }
}

#[test]
fn committed_merge_recovers_journal_only_ack_and_checks_scope_before_payload_reads() {
    let (_project, mut repo, parent) = fixture();
    let before = repo.status().unwrap();
    repo.connection.wal_hook(Some(|_, _| {
        Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_IOERR),
            None,
        ))
    }));
    assert!(
        repo.commit_merge(parent, "Author", "merge", 2, "merge")
            .is_err()
    );
    repo.connection.wal_hook(None);
    let Receipt::Selection(original) = repo.receipt("merge").unwrap().unwrap() else {
        panic!("selection receipt")
    };
    assert_eq!(original.operation, Operation::MergeCommit { parent });
    assert_eq!(original.before, before);
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
        repo.receipt("merge").unwrap(),
        Some(Receipt::Selection(original.clone()))
    );
    assert!(matches!(
        repo.commit("Author", "merge", 2, "merge"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert!(matches!(
        repo.commit_merge(before.revision.unwrap(), "Author", "merge", 2, "merge"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert!(matches!(
        repo.commit_merge(parent, "Author", "merge", 3, "merge"),
        Err(Error::RequestKeyReuse(_))
    ));
    repo.connection
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .unwrap();
    assert_eq!(
        repo.commit_merge(parent, "Author", "merge", 2, "merge")
            .unwrap(),
        original
    );
}

#[test]
fn merge_sql_and_receipt_decoder_require_typed_parent_and_exact_selection_shape() {
    let (_project, mut repo, parent) = fixture();
    repo.commit_merge(parent, "Author", "merge", 2, "merge")
        .unwrap();
    for (i, (kind, before_head, after_head, root, extra)) in [
        (
            "'merge-commit'",
            "before_revision",
            "after_revision",
            "after_root",
            "NULL",
        ),
        (
            "'merge-commit'",
            "before_revision",
            "after_revision",
            "after_root",
            "X'05'",
        ),
        (
            "'merge-commit'",
            "before_revision",
            "after_revision",
            "after_root",
            "before_root",
        ),
        (
            "'merge-commit'",
            "NULL",
            "after_revision",
            "after_root",
            "merge_parent",
        ),
        (
            "'merge-commit'",
            "before_revision",
            "NULL",
            "after_root",
            "merge_parent",
        ),
        (
            "'merge-commit'",
            "before_revision",
            "before_revision",
            "after_root",
            "merge_parent",
        ),
        (
            "'merge-commit'",
            "before_revision",
            "after_revision",
            "after_revision",
            "merge_parent",
        ),
        (
            "'merge-commit'",
            "before_revision",
            "after_revision",
            "after_root",
            "before_revision",
        ),
        (
            "'commit'",
            "before_revision",
            "after_revision",
            "after_root",
            "merge_parent",
        ),
        (
            "'amend'",
            "before_revision",
            "after_revision",
            "after_root",
            "merge_parent",
        ),
        (
            "'select'",
            "before_revision",
            "after_revision",
            "after_root",
            "merge_parent",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let key = format!("bad-{i}");
        let sql = format!(
            "INSERT INTO operations (request_key, kind, before_root, before_generation, before_revision, after_root, after_generation, after_revision, merge_parent) SELECT ?1, {kind}, before_root, before_generation, {before_head}, {root}, after_generation, {after_head}, {extra} FROM operations WHERE request_key = 'merge'"
        );
        assert!(
            repo.connection.execute(&sql, [&key]).is_err(),
            "SQL accepted {i}"
        );
        // Simulate retained metadata damage without filesystem descriptors.
        repo.connection
            .pragma_update(None, "ignore_check_constraints", true)
            .unwrap();
        repo.connection
            .pragma_update(None, "foreign_keys", false)
            .unwrap();
        repo.connection.execute(&sql, [&key]).unwrap();
        repo.connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        assert!(repo.receipt(&key).is_err(), "decoder accepted {i}");
        repo.connection
            .pragma_update(None, "ignore_check_constraints", false)
            .unwrap();
    }
    let sql = "INSERT INTO operations (request_key, kind, ref_name, after_ref_generation, after_ref_revision, merge_parent) VALUES ('bad-ref', 'ref-set', 'saved', 1, ?1, ?1)";
    let bytes = parent.to_bytes().unwrap();
    assert!(repo.connection.execute(sql, [bytes.as_slice()]).is_err());
    repo.connection
        .pragma_update(None, "ignore_check_constraints", true)
        .unwrap();
    repo.connection.execute(sql, [bytes.as_slice()]).unwrap();
    assert!(repo.receipt("bad-ref").is_err());
}
