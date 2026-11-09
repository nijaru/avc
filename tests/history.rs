use avc::{
    Entry, Id, Object,
    engine::{Edit, Engine},
    repository::{Error, MAX_OBJECTS, Operation, Repository},
};
use std::sync::{Arc, Barrier};

fn source(bytes: &[u8]) -> (Engine, Id) {
    let mut engine = Engine::new();
    let root = engine.empty().unwrap();
    let blob = engine.blob(bytes.to_vec()).unwrap();
    let root = engine
        .edit(
            root,
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
    (engine, root)
}

#[test]
fn explicit_commits_link_authored_history_without_capturing_source_edits() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    assert_eq!(repo.status().unwrap().revision, None);
    assert!(repo.log(None, 20).unwrap().is_empty());
    let (engine, first_root) = source(b"first");
    let source_receipt = repo
        .commit_source(&engine, first_root, 0, "source-one")
        .unwrap();
    assert_eq!(source_receipt.operation, Operation::Source);
    assert_eq!(source_receipt.after.revision, None);
    let first = repo
        .commit("Ada <ada@example.test>", "first commit", 1, "commit-one")
        .unwrap();
    assert_eq!(first.operation, Operation::Commit);
    assert_eq!(first.before, source_receipt.after);
    assert_eq!(first.after.root, first_root);
    assert_eq!(first.after.generation, 2);
    let first_id = first.after.revision.unwrap();
    let revision = repo.revision(first_id).unwrap();
    assert_eq!(revision.root, first_root);
    assert!(revision.parents.is_empty());
    assert_eq!(revision.author, "Ada <ada@example.test>");
    assert_eq!(revision.message, "first commit");
    assert_eq!(revision.timezone_minutes, 0);
    assert!(revision.time_ns > 0);
    repo.load_root(first_id)
        .unwrap()
        .verify(first_id, Default::default())
        .unwrap();

    let (engine, second_root) = source(b"second");
    let edited = repo
        .commit_source(&engine, second_root, 2, "source-two")
        .unwrap();
    assert_eq!(edited.after.revision, Some(first_id));
    assert_eq!(repo.log(None, 20).unwrap().len(), 1);
    let second = repo
        .commit("Grace", "second commit", 3, "commit-two")
        .unwrap();
    let second_id = second.after.revision.unwrap();
    let second_revision = repo.revision(second_id).unwrap();
    assert_eq!(second_revision.parents, vec![first_id]);
    assert_eq!(second_revision.root, second_root);
    assert_ne!(second_revision.change, revision.change);
    assert_eq!(
        repo.log(None, 20)
            .unwrap()
            .iter()
            .map(|r| r.0)
            .collect::<Vec<_>>(),
        vec![second_id, first_id]
    );
    assert_eq!(repo.log(None, 1).unwrap()[0].0, second_id);
    assert_eq!(
        repo.log(Some(first_id), 20).unwrap(),
        vec![(first_id, revision)]
    );
    drop(repo);
    let repo = Repository::open(project.path()).unwrap();
    assert_eq!(repo.status().unwrap(), second.after);
    assert_eq!(repo.log(None, 20).unwrap().len(), 2);
    repo.load_root(second_id)
        .unwrap()
        .verify(second_id, Default::default())
        .unwrap();
}

#[test]
fn commit_retries_preserve_generated_identity_and_reject_cross_operation_key_reuse() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    // An explicit empty-source revision is meaningful; init itself authored none.
    let first = repo.commit("Author", "", 0, "one").unwrap();
    let original = repo.revision(first.after.revision.unwrap()).unwrap();
    let (engine, root) = source(b"later source");
    repo.commit_source(&engine, root, 1, "edit").unwrap();
    let latest = repo.commit("Author", "later", 2, "two").unwrap();
    assert_eq!(repo.commit("Author", "", 0, "one").unwrap(), first);
    assert_eq!(
        repo.revision(first.after.revision.unwrap()).unwrap(),
        original
    );
    assert_eq!(repo.status().unwrap(), latest.after);
    for (author, message) in [("", "".to_owned()), ("Author", "x".repeat(1024 * 1024 + 1))] {
        assert!(matches!(
            repo.commit(author, &message, 0, "one"),
            Err(Error::RequestKeyReuse(_))
        ));
    }
    for (author, message, generation) in [
        ("Other", "", 0),
        ("Author", "different", 0),
        ("Author", "", 2),
    ] {
        assert!(matches!(
            repo.commit(author, message, generation, "one"),
            Err(Error::RequestKeyReuse(_))
        ));
    }
    assert!(matches!(
        repo.commit("Author", "x", 1, "edit"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert!(matches!(
        repo.commit_source(&Engine::new(), first.after.root, 0, "one"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert!(matches!(
        repo.preparation_base(0, "one"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert_eq!(repo.status().unwrap(), latest.after);
}

#[test]
fn concurrent_authors_use_one_generation_and_identical_requests_share_one_revision() {
    for identical in [false, true] {
        let project = tempfile::tempdir().unwrap();
        drop(Repository::init(project.path()).unwrap());
        let barrier = Arc::new(Barrier::new(2));
        let workers = (0..2)
            .map(|i| {
                let mut repo = Repository::open(project.path()).unwrap();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    repo.commit(
                        "Author",
                        "commit",
                        0,
                        if identical || i == 0 { "one" } else { "two" },
                    )
                })
            })
            .collect::<Vec<_>>();
        let results = workers
            .into_iter()
            .map(|w| w.join().unwrap())
            .collect::<Vec<_>>();
        if identical {
            assert_eq!(results[0].as_ref().unwrap(), results[1].as_ref().unwrap());
        } else {
            assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
            assert_eq!(
                results
                    .iter()
                    .filter(|r| matches!(
                        r,
                        Err(Error::Stale {
                            expected: 0,
                            actual: 1
                        })
                    ))
                    .count(),
                1
            );
        }
        let repo = Repository::open(project.path()).unwrap();
        assert_eq!(repo.status().unwrap().generation, 1);
        assert_eq!(repo.log(None, 20).unwrap().len(), 1);
    }
}

#[test]
fn rejected_commit_rolls_back_revision_head_and_receipt_together() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let before = repo.status().unwrap();
    let db = rusqlite::Connection::open(project.path().join(".avc/store.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER reject_commit BEFORE INSERT ON operations WHEN NEW.kind = 'commit' BEGIN SELECT RAISE(ABORT, 'denied commit receipt'); END;").unwrap();
    assert!(repo.commit("Author", "one", 0, "one").is_err());
    assert_eq!(repo.status().unwrap(), before);
    assert!(repo.receipt("one").unwrap().is_none());
    assert!(repo.log(None, 20).unwrap().is_empty());
    assert_eq!(
        db.query_row::<i64, _, _>("SELECT count(*) FROM objects", [], |r| r.get(0))
            .unwrap(),
        1
    );
    db.execute_batch("DROP TRIGGER reject_commit").unwrap();
    let committed = repo.commit("Author", "one", 0, "one").unwrap();
    assert_eq!(repo.commit("Author", "one", 0, "one").unwrap(), committed);
    assert_eq!(repo.log(None, 20).unwrap().len(), 1);
}

#[test]
fn metadata_limits_and_source_only_transfer_have_explicit_boundaries() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let before = repo.status().unwrap();
    for author in ["".to_owned(), " \n".to_owned(), "x".repeat(4097)] {
        assert!(repo.commit(&author, "message", 0, "bad").is_err());
    }
    assert!(
        repo.commit("Author", &"x".repeat(1024 * 1024 + 1), 0, "bad")
            .is_err()
    );
    assert_eq!(repo.status().unwrap(), before);
    let (engine, root) = source(b"exact selected bytes");
    repo.commit_source(&engine, root, 0, "source").unwrap();
    let revision = repo
        .commit("Author", "message", 1, "commit")
        .unwrap()
        .after
        .revision
        .unwrap();
    assert!(repo.log(None, 0).unwrap().is_empty());
    assert!(matches!(
        repo.log(None, MAX_OBJECTS + 1),
        Err(Error::Limit("history records"))
    ));
    assert!(repo.log(Some(root), 1).is_err());
    assert!(repo.revision(root).is_err());
    let mut rejected = Vec::new();
    assert!(repo.export(revision, &mut rejected).is_err());
    assert!(rejected.is_empty());
    let Entry::File { blob, .. } = engine.read(root, &[b"file".to_vec()]).unwrap() else {
        panic!()
    };
    for id in [revision, blob] {
        let path = project.path().join("must-not-create.avc");
        assert!(repo.export_file(id, &path).is_err());
        assert!(
            !path.exists(),
            "invalid source kind must refuse before creating an export"
        );
    }
    let mut bytes = Vec::new();
    repo.export(root, &mut bytes).unwrap();
    let fresh = tempfile::tempdir().unwrap();
    let mut restored = Repository::init(fresh.path()).unwrap();
    restored.import(bytes.as_slice(), "import").unwrap();
    assert_eq!(restored.status().unwrap().root, root);
    assert_eq!(restored.status().unwrap().revision, None);
    assert!(restored.log(None, 20).unwrap().is_empty());
    let loaded = restored.load_root(root).unwrap();
    let Entry::File { blob, .. } = loaded.read(root, &[b"file".to_vec()]).unwrap() else {
        panic!()
    };
    assert_eq!(
        loaded.object(blob).unwrap(),
        &Object::Blob(b"exact selected bytes".to_vec())
    );
}
