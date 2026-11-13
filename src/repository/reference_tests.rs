use super::*;

fn fixture() -> (tempfile::TempDir, Repository, Id) {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let revision = repo
        .commit("Author", "message", 0, "author")
        .unwrap()
        .after
        .revision
        .unwrap();
    (project, repo, revision)
}

fn insert(
    connection: &Connection,
    key: &str,
    name: &str,
    before: Option<i64>,
    after: i64,
    revision: Id,
) -> rusqlite::Result<usize> {
    connection.execute(
        "INSERT INTO operations (request_key, kind, ref_name, before_ref_generation, after_ref_generation, after_ref_revision) VALUES (?1, 'ref-set', ?2, ?3, ?4, ?5)",
        params![key, name, before, after, revision.to_bytes().unwrap().as_slice()],
    )
}

#[test]
fn journal_enforces_disjoint_shapes_and_explicit_presence() {
    let (_project, repo, revision) = fixture();
    let root = repo.status().unwrap().root.to_bytes().unwrap();
    let revision = revision.to_bytes().unwrap();
    for (index, column) in ["ref_name", "after_ref_generation", "after_ref_revision"]
        .iter()
        .enumerate()
    {
        let sql = format!(
            "INSERT INTO operations (request_key, kind, ref_name, after_ref_generation, after_ref_revision) VALUES ('missing-{index}', 'ref-set', {}, {}, {})",
            if *column == "ref_name" {
                "NULL"
            } else {
                "'main'"
            },
            if *column == "after_ref_generation" {
                "NULL"
            } else {
                "1"
            },
            if *column == "after_ref_revision" {
                "NULL"
            } else {
                "?1"
            }
        );
        // All variants retain a bound parameter, including NULL-target cases.
        let sql = format!("{sql} RETURNING ?1");
        assert!(
            repo.connection
                .query_row(&sql, [revision.as_slice()], |_| Ok(()))
                .is_err(),
            "{column}"
        );
    }
    for column in [
        "before_root",
        "before_generation",
        "after_root",
        "after_generation",
    ] {
        let sql = format!(
            "INSERT INTO operations (request_key, kind, before_root, before_generation, after_root, after_generation) VALUES ('missing-{column}', 'source', {}, {}, {}, {})",
            if column == "before_root" {
                "NULL"
            } else {
                "?1"
            },
            if column == "before_generation" {
                "NULL"
            } else {
                "0"
            },
            if column == "after_root" { "NULL" } else { "?1" },
            if column == "after_generation" {
                "NULL"
            } else {
                "1"
            }
        );
        assert!(
            repo.connection.execute(&sql, [root.as_slice()]).is_err(),
            "{column}"
        );
    }
    for (column, value) in [
        ("before_root", "?2"),
        ("before_generation", "0"),
        ("before_revision", "?1"),
        ("after_root", "?2"),
        ("after_generation", "1"),
        ("after_revision", "?1"),
        ("undo_target", "'author'"),
        ("onto_revision", "?1"),
        ("git_commit", "zeroblob(21)"),
        ("capture_scope", "X'01'"),
        ("import_origins", "zeroblob(44)"),
    ] {
        let sql = format!(
            "INSERT INTO operations (request_key, kind, ref_name, after_ref_generation, after_ref_revision, {column}) VALUES ('extra-{column}', 'ref-set', 'main', 1, ?1, {value}) RETURNING ?2"
        );
        assert!(
            repo.connection
                .query_row(&sql, params![revision.as_slice(), root.as_slice()], |_| Ok(
                    ()
                ))
                .is_err(),
            "{column}"
        );
    }
    for (column, value) in [
        ("ref_name", "'main'"),
        ("before_ref_generation", "1"),
        ("after_ref_generation", "1"),
        ("after_ref_revision", "?2"),
    ] {
        let sql = format!(
            "INSERT INTO operations (request_key, kind, before_root, before_generation, after_root, after_generation, {column}) VALUES ('extra-{column}', 'source', ?1, 0, ?1, 1, {value}) RETURNING ?2"
        );
        assert!(
            repo.connection
                .query_row(&sql, params![root.as_slice(), revision.as_slice()], |_| Ok(
                    ()
                ))
                .is_err(),
            "{column}"
        );
    }
    let count: i64 = repo
        .connection
        .query_row("SELECT count(*) FROM operations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn journal_requires_rooted_unforked_finite_chains_and_revision_foreign_keys() {
    let (_project, repo, revision) = fixture();
    let db = &repo.connection;
    for (key, before, after) in [
        ("gap-birth", None, 2),
        ("zero", None, 0),
        ("negative", None, -1),
        ("zero-before", Some(0), 1),
        ("negative-before", Some(-1), 0),
        ("missing", Some(1), 2),
        ("overflow", Some(i64::MAX), i64::MAX),
    ] {
        assert!(
            insert(db, key, "main", before, after, revision).is_err(),
            "{key}"
        );
    }
    insert(db, "birth", "main", None, 1, revision).unwrap();
    insert(db, "second", "main", Some(1), 2, revision).unwrap();
    assert!(insert(db, "fork", "main", Some(1), 2, revision).is_err());
    assert!(insert(db, "rebirth", "main", None, 1, revision).is_err());
    assert!(insert(db, "gap", "main", Some(2), 4, revision).is_err());
    assert!(insert(db, "other-name", "other", Some(2), 3, revision).is_err());
    assert!(
        insert(
            db,
            "wrong-kind",
            "other",
            None,
            1,
            repo.status().unwrap().root
        )
        .is_err()
    );
    let missing = Object::Revision(crate::Revision {
        root: repo.status().unwrap().root,
        parents: Vec::new(),
        change: [9; 16],
        author: "Missing".into(),
        message: String::new(),
        time_ns: 0,
        timezone_minutes: 0,
    })
    .id()
    .unwrap();
    assert!(insert(db, "missing-target", "other", None, 1, missing).is_err());
    assert!(
        db.execute(
            "UPDATE operations SET after_ref_generation = 3 WHERE request_key = 'second'",
            []
        )
        .is_err()
    );
    assert!(
        db.execute("DELETE FROM operations WHERE request_key = 'second'", [])
            .is_err()
    );
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM operations WHERE ref_name = 'main'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn page_plan_ranges_birth_index_and_seeks_latest_generation() {
    let (_project, mut repo, revision) = fixture();
    for name in ["a", "a/b", "b", "é"] {
        let name = name.parse().unwrap();
        for generation in 0..3 {
            repo.set_reference(
                &name,
                revision,
                generation,
                &format!("{name:?}-{generation}"),
            )
            .unwrap();
        }
    }
    let mut statement = repo
        .connection
        .prepare(&format!("EXPLAIN QUERY PLAN {LIST}"))
        .unwrap();
    let plan = statement
        .query_map(params!["a", 2], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert!(plan.iter().any(|line| line.contains("SEARCH birth USING INDEX operations_ref_birth (ref_name>?)")), "{plan:?}");
    assert!(
        plan.iter().any(|line| line
            .contains("SEARCH tip USING COVERING INDEX operations_ref_generation (ref_name=?)")),
        "{plan:?}"
    );
    assert!(plan.iter().any(|line| line.contains("SEARCH latest USING INDEX operations_ref_generation (ref_name=? AND after_ref_generation=?)")), "{plan:?}");
    assert!(
        !plan
            .iter()
            .any(|line| line.contains("SCAN") || line.contains("TEMP B-TREE")),
        "{plan:?}"
    );
    let page = repo.references(Some(&"a".parse().unwrap()), 2).unwrap();
    assert_eq!(
        page.iter()
            .map(|named| (named.name.as_str(), named.value.generation))
            .collect::<Vec<_>>(),
        [("a/b", 3), ("b", 3)]
    );
}

#[test]
fn selection_callers_refuse_ref_owned_keys_before_reading_inputs() {
    let (_project, mut repo, revision) = fixture();
    let name = "main".parse().unwrap();
    repo.set_reference(&name, revision, 0, "ref").unwrap();
    let selected = repo.status().unwrap();
    let check = |error| assert!(matches!(error, Error::RequestKeyReuse(key) if key == "ref"));
    check(
        repo.preparation_base(selected.generation, "ref")
            .unwrap_err(),
    );
    check(
        repo.commit_source(&Engine::new(), selected.root, selected.generation, "ref")
            .unwrap_err(),
    );
    check(repo.commit("", "", selected.generation, "ref").unwrap_err());
    check(repo.amend("", "", selected.generation, "ref").unwrap_err());
    check(
        repo.rebase(revision, selected.generation, "ref")
            .unwrap_err(),
    );
    check(
        repo.undo("nonexistent", selected.generation, "ref")
            .unwrap_err(),
    );
    let scope = CaptureScope::new(Path::new("/nonexistent"), vec![vec![b"file".to_vec()]]).unwrap();
    check(
        repo.capture(&scope, selected.generation, "ref")
            .unwrap_err(),
    );
    check(repo.import(&b"invalid"[..], "ref").unwrap_err());
    check(repo.import_history(&b"invalid"[..], "ref").unwrap_err());
    let commit = crate::git::GitOid::from_bytes(&[&[1][..], &[0; 20][..]].concat()).unwrap();
    check(
        repo.import_git(Path::new("/nonexistent"), commit, "ref")
            .unwrap_err(),
    );
    assert!(matches!(
        repo.set_reference(&name, revision, 0, "author"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert!(repo.capture_observation("ref").unwrap().is_none());
    assert!(
        matches!(repo.undo("ref", selected.generation, "undo-ref"), Err(Error::Invalid(message)) if message.contains("selection undo cannot target a reference"))
    );
    assert_eq!(repo.status().unwrap(), selected);
}

#[test]
fn reference_publication_never_reads_selection_or_writes_other_authority() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};

    let (_project, mut repo, revision) = fixture();
    let selected = repo.status().unwrap();
    let name = "main".parse().unwrap();
    repo.connection.authorizer(Some(|ctx: AuthContext<'_>| {
        match ctx.action {
            // SQLite authorizes all FK parent reads when compiling INSERT,
            // even the NULL git_commit FK; only selection reads are forbidden.
            AuthAction::Read { table_name: "selection", .. }
            | AuthAction::Insert { table_name: "objects" | "selection" | "git_origins" | "git_exports" | "capture_observations" }
            | AuthAction::Update { .. }
            | AuthAction::Delete { .. } => Authorization::Deny,
            _ => Authorization::Allow,
        }
    })).unwrap();
    repo.set_reference(&name, revision, 0, "first").unwrap();
    let second = repo.set_reference(&name, revision, 1, "second").unwrap();
    repo.connection
        .authorizer(Some(|ctx: AuthContext<'_>| match ctx.action {
            AuthAction::Read {
                table_name: "objects" | "selection" | "git_origins" | "git_exports",
                ..
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }))
        .unwrap();
    assert_eq!(repo.reference(&name).unwrap(), Some(second.after));
    assert_eq!(
        repo.references(None, 20).unwrap(),
        [NamedRef {
            name: name.clone(),
            value: second.after
        }]
    );
    assert_eq!(
        repo.receipt("second").unwrap(),
        Some(Receipt::Ref(second.clone()))
    );
    assert_eq!(
        repo.set_reference(&name, revision, 1, "second").unwrap(),
        second
    );
    repo.connection
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .unwrap();
    assert_eq!(repo.status().unwrap(), selected);
}

#[test]
fn all_reference_entry_points_fence_unfinished_transactions() {
    let (_project, mut repo, revision) = fixture();
    let name = "main".parse().unwrap();
    repo.connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert!(matches!(
        repo.reference(&name),
        Err(Error::RecoveryRequired)
    ));
    assert!(matches!(
        repo.references(None, 0),
        Err(Error::RecoveryRequired)
    ));
    assert!(matches!(
        repo.set_reference(&name, revision, 0, "set"),
        Err(Error::RecoveryRequired)
    ));
    assert!(matches!(repo.receipt("set"), Err(Error::RecoveryRequired)));
    repo.connection.execute_batch("ROLLBACK").unwrap();
    assert!(repo.reference(&name).unwrap().is_none());
}

#[test]
fn damaged_missing_predecessor_is_not_a_creation_receipt() {
    let (_project, mut repo, revision) = fixture();
    let name = "main".parse().unwrap();
    repo.set_reference(&name, revision, 0, "first").unwrap();
    repo.set_reference(&name, revision, 1, "second").unwrap();
    repo.connection.execute_batch("PRAGMA foreign_keys = OFF; DROP TRIGGER operations_no_delete; DELETE FROM operations WHERE request_key = 'first'; PRAGMA foreign_keys = ON").unwrap();
    assert!(
        matches!(repo.receipt("second"), Err(Error::Invalid(message)) if message.contains("missing reference receipt predecessor"))
    );
    assert!(
        matches!(repo.set_reference(&name, revision, 1, "second"), Err(Error::Invalid(message)) if message.contains("missing reference receipt predecessor"))
    );
}

#[test]
fn exhausted_reference_generation_is_explicit_and_never_wraps() {
    let (_project, mut repo, revision) = fixture();
    // Model otherwise-unreachable exhaustion without authoring i64::MAX rows.
    repo.connection
        .execute_batch("PRAGMA ignore_check_constraints = ON")
        .unwrap();
    insert(
        &repo.connection,
        "exhausted",
        "main",
        None,
        i64::MAX,
        revision,
    )
    .unwrap();
    repo.connection
        .execute_batch("PRAGMA ignore_check_constraints = OFF")
        .unwrap();
    assert!(matches!(
        repo.set_reference(&"main".parse().unwrap(), revision, i64::MAX as u64, "new"),
        Err(Error::Limit("reference generation exhausted"))
    ));
    assert!(repo.receipt("new").unwrap().is_none());
}

#[test]
fn recovered_reference_ack_fences_after_releasing_write_transaction() {
    let (project, mut repo, revision) = fixture();
    let name = "main".parse().unwrap();
    let original = repo.set_reference(&name, revision, 0, "first").unwrap();
    let reader = Connection::open(project.path().join(".avc/store.sqlite")).unwrap();
    reader
        .execute_batch("BEGIN; SELECT count(*) FROM operations")
        .unwrap();
    repo.set_reference(&name, revision, 1, "second").unwrap();
    repo.connection.busy_timeout(Duration::ZERO).unwrap();
    // Wrong identities never acknowledge anything and need no checkpoint fence.
    assert!(matches!(
        repo.set_reference(&name, revision, 1, "first"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert!(matches!(
        repo.set_reference(&name, revision, 0, "author"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert!(matches!(
        repo.commit("Author", "message", 1, "first"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert!(matches!(
        repo.set_reference(&name, revision, 0, "first"),
        Err(Error::DurabilityPending)
    ));
    assert!(repo.connection.is_autocommit());
    assert!(matches!(
        repo.receipt("first"),
        Err(Error::DurabilityPending)
    ));
    reader.execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        repo.set_reference(&name, revision, 0, "first").unwrap(),
        original
    );
}

#[test]
fn reference_commit_failure_recovers_only_the_committed_journal_row() {
    let (_project, mut repo, revision) = fixture();
    let name = "main".parse().unwrap();
    repo.connection.commit_hook(Some(|| true)).unwrap();
    assert!(repo.set_reference(&name, revision, 0, "first").is_err());
    repo.connection.commit_hook(None::<fn() -> bool>).unwrap();
    assert!(repo.reference(&name).unwrap().is_none());
    assert!(repo.receipt("first").unwrap().is_none());
    repo.connection.wal_hook(Some(|_, _| {
        Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_IOERR),
            None,
        ))
    }));
    assert!(repo.set_reference(&name, revision, 0, "first").is_err());
    repo.connection.wal_hook(None);
    let Receipt::Ref(committed) = repo.receipt("first").unwrap().unwrap() else {
        panic!("expected ref receipt")
    };
    assert_eq!(committed.after.generation, 1);
    assert_eq!(
        repo.set_reference(&name, revision, 0, "first").unwrap(),
        committed
    );
}
