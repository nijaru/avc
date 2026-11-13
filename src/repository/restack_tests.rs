use super::*;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn fixture() -> (tempfile::TempDir, Repository, Id, Id) {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let mut revisions = Vec::new();
    for (generation, key) in ["root", "base", "middle", "tip"].into_iter().enumerate() {
        revisions.push(
            repo.commit("Author", key, generation as u64, key)
                .unwrap()
                .after
                .revision
                .unwrap(),
        );
    }
    (project, repo, revisions[1], revisions[0])
}
fn object_count(repo: &Repository) -> i64 {
    repo.connection
        .query_row("SELECT count(*) FROM objects", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn whole_range_rolls_back_after_partial_payload_insert_or_selection_receipt_commit_failure() {
    for boundary in ["objects", "selection", "operations", "commit"] {
        let (project, mut repo, base, onto) = fixture();
        let before = repo.status().unwrap();
        let count = object_count(&repo);
        let observer = Repository::open(project.path()).unwrap();
        let inserted = Arc::new(AtomicUsize::new(0));
        if boundary == "commit" {
            repo.connection.commit_hook(Some(|| true)).unwrap();
        } else {
            let inserted = inserted.clone();
            repo.connection
                .authorizer(Some(move |ctx: AuthContext<'_>| match ctx.action {
                    AuthAction::Insert {
                        table_name: "objects",
                    } if boundary == "objects" => {
                        if inserted.fetch_add(1, Ordering::Relaxed) == 1 {
                            Authorization::Deny
                        } else {
                            Authorization::Allow
                        }
                    }
                    AuthAction::Insert {
                        table_name: "operations",
                    } if boundary == "operations" => Authorization::Deny,
                    AuthAction::Update {
                        table_name: "selection",
                        ..
                    } if boundary == "selection" => Authorization::Deny,
                    _ => Authorization::Allow,
                }))
                .unwrap();
        }
        assert!(
            repo.restack(base, onto, 4, "restack").is_err(),
            "{boundary}"
        );
        repo.connection
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .unwrap();
        repo.connection.commit_hook(None::<fn() -> bool>).unwrap();
        if boundary == "objects" {
            assert_eq!(inserted.load(Ordering::Relaxed), 2);
        }
        assert_eq!(repo.status().unwrap(), before);
        assert_eq!(observer.status().unwrap(), before);
        assert_eq!(object_count(&repo), count);
        assert!(repo.receipt("restack").unwrap().is_none());
        let receipt = repo.restack(base, onto, 4, "restack").unwrap();
        assert_eq!(receipt.before, before);
        assert_eq!(object_count(&repo), count + 2);
        let Operation::Restack { rewrites, .. } = receipt.operation else {
            panic!("restack")
        };
        assert_eq!(rewrites.len(), 2);
        assert_eq!(repo.revision(rewrites[0].after).unwrap().parents, [onto]);
        assert_eq!(
            repo.revision(rewrites[1].after).unwrap().parents,
            [rewrites[0].after]
        );
    }
}

#[test]
fn committed_wal_error_recovers_complete_scope_without_any_graph_or_current_selection_reads() {
    let (_project, mut repo, base, onto) = fixture();
    repo.connection.wal_hook(Some(|_, _| {
        Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_IOERR),
            None,
        ))
    }));
    assert!(repo.restack(base, onto, 4, "restack").is_err());
    repo.connection.wal_hook(None);
    let Receipt::Selection(original) = repo.receipt("restack").unwrap().unwrap() else {
        panic!("selection")
    };
    repo.connection
        .authorizer(Some(|ctx: AuthContext<'_>| match ctx.action {
            AuthAction::Read {
                table_name: "objects" | "selection" | "git_origins" | "git_exports",
                ..
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }))
        .unwrap();
    assert_eq!(repo.restack(base, onto, 4, "restack").unwrap(), original);
    assert_eq!(
        repo.receipt("restack").unwrap(),
        Some(Receipt::Selection(original))
    );
    for (b, o, generation) in [(onto, onto, 4), (base, base, 4), (base, onto, 5)] {
        assert!(matches!(
            repo.restack(b, o, generation, "restack"),
            Err(Error::RequestKeyReuse(_))
        ));
    }
    assert!(matches!(
        repo.commit("Author", "irrelevant", 4, "restack"),
        Err(Error::RequestKeyReuse(_))
    ));
}

#[test]
fn restack_retry_obeys_the_checked_full_checkpoint_fence() {
    let (project, mut repo, base, onto) = fixture();
    let original = repo.restack(base, onto, 4, "restack").unwrap();
    let reader = Connection::open(project.path().join(".avc/store.sqlite")).unwrap();
    reader
        .execute_batch("BEGIN; SELECT count(*) FROM operations")
        .unwrap();
    repo.commit("Author", "later", 5, "later").unwrap();
    repo.connection.busy_timeout(Duration::ZERO).unwrap();
    assert!(matches!(
        repo.restack(base, onto, 4, "restack"),
        Err(Error::DurabilityPending)
    ));
    assert!(repo.connection.is_autocommit());
    reader.execute_batch("ROLLBACK").unwrap();
    assert_eq!(repo.restack(base, onto, 4, "restack").unwrap(), original);
}

#[test]
fn sql_scope_iff_and_head_presence_are_enforced_and_decoder_binds_exact_endpoints() {
    let (_project, mut repo, base, onto) = fixture();
    repo.restack(base, onto, 4, "restack").unwrap();
    for (i, (kind, before, after, scope)) in [
        ("'restack'", "before_revision", "after_revision", "NULL"),
        ("'restack'", "before_revision", "after_revision", "X'00'"),
        ("'restack'", "NULL", "after_revision", "restack_scope"),
        ("'restack'", "before_revision", "NULL", "restack_scope"),
        (
            "'select'",
            "before_revision",
            "after_revision",
            "restack_scope",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let key = format!("bad-{i}");
        let sql = format!(
            "INSERT INTO operations (request_key, kind, before_root, before_generation, before_revision, after_root, after_generation, after_revision, restack_scope) SELECT ?1, {kind}, before_root, before_generation, {before}, after_root, after_generation, {after}, {scope} FROM operations WHERE request_key = 'restack'"
        );
        assert!(repo.connection.execute(&sql, [&key]).is_err(), "SQL {i}");
        repo.connection
            .pragma_update(None, "ignore_check_constraints", true)
            .unwrap();
        repo.connection.execute(&sql, [&key]).unwrap();
        assert!(repo.receipt(&key).is_err(), "decoder {i}");
        repo.connection
            .pragma_update(None, "ignore_check_constraints", false)
            .unwrap();
    }
    // Both are legal revision FKs; the private scope still binds different heads.
    repo.connection.execute("INSERT INTO operations (request_key, kind, before_root, before_generation, before_revision, after_root, after_generation, after_revision, restack_scope) SELECT 'wrong-endpoint', kind, before_root, before_generation, ?1, after_root, after_generation, after_revision, restack_scope FROM operations WHERE request_key = 'restack'", [base.to_bytes().unwrap().as_slice()]).unwrap();
    assert!(repo.receipt("wrong-endpoint").is_err());
    let scope: Vec<u8> = repo
        .connection
        .query_row(
            "SELECT restack_scope FROM operations WHERE request_key = 'restack'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let sql = "INSERT INTO operations (request_key, kind, ref_name, after_ref_generation, after_ref_revision, restack_scope) VALUES ('bad-ref', 'ref-set', 'saved', 1, ?1, ?2)";
    assert!(
        repo.connection
            .execute(sql, params![base.to_bytes().unwrap().as_slice(), &scope])
            .is_err()
    );
    repo.connection
        .pragma_update(None, "ignore_check_constraints", true)
        .unwrap();
    repo.connection
        .execute(sql, params![base.to_bytes().unwrap().as_slice(), &scope])
        .unwrap();
    assert!(repo.receipt("bad-ref").is_err());
}

#[test]
fn failed_closure_extension_is_atomic_and_existing_ownership_never_refetches() {
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let blob = engine.blob(b"new".to_vec()).unwrap();
    let root = engine
        .edit(
            empty,
            &[crate::engine::Edit {
                path: vec![b"file".to_vec()],
                expected: Entry::Absent,
                value: Entry::File {
                    blob,
                    executable: false,
                },
            }],
        )
        .unwrap();
    let mut output = ClosureBuilder::default();
    output
        .extend([empty], |id| encode_bounded(engine.object(id)?))
        .unwrap();
    let before = output.records.clone();
    let bytes = output.bytes;
    assert!(
        output
            .extend([root], |id| if id == blob {
                Err(Error::MissingObject(id))
            } else {
                encode_bounded(engine.object(id)?)
            })
            .is_err()
    );
    assert_eq!(output.records, before);
    assert_eq!(output.bytes, bytes);
    output
        .extend([root], |id| encode_bounded(engine.object(id)?))
        .unwrap();
    output
        .extend([empty, root, blob], |_| panic!("already owned"))
        .unwrap();
}

#[test]
fn cumulative_closure_bytes_charge_reused_input_and_do_not_reset_at_each_step() {
    let mut output = ClosureBuilder::default();
    // Each supplier is an already available input blob. Admission still charges
    // its complete bytes, not just objects newly generated by a replay step.
    for tag in 0..16 {
        let object = Object::Blob(vec![tag; MAX_OBJECT_BYTES]);
        let id = object.id().unwrap();
        output.extend([id], |_| encode_bounded(&object)).unwrap();
    }
    assert_eq!(output.bytes, MAX_CLOSURE_BYTES);
    let extra = Object::Blob(vec![255]);
    let id = extra.id().unwrap();
    assert!(matches!(
        output.extend([id], |_| encode_bounded(&extra)),
        Err(Error::Limit("closure bytes"))
    ));
    assert!(!output.records.contains_key(&id));
    assert_eq!(output.bytes, MAX_CLOSURE_BYTES);
}

#[test]
fn cumulative_closure_object_limit_rejects_new_roots_before_fetching() {
    let input: BTreeMap<_, _> = (0..MAX_OBJECTS as u32)
        .map(|n| {
            let object = Object::Blob(n.to_le_bytes().to_vec());
            (object.id().unwrap(), object.encode().unwrap())
        })
        .collect();
    let mut output = ClosureBuilder::default();
    output
        .extend(input.keys().copied(), |id| Ok(input[&id].clone()))
        .unwrap();
    let extra = Object::Blob(b"extra".to_vec()).id().unwrap();
    assert!(matches!(
        output.extend([extra], |_| panic!("over budget before fetch")),
        Err(Error::Limit("closure objects"))
    ));
    assert_eq!(output.records.len(), MAX_OBJECTS);
}
