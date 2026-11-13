use avc::{
    Entry, Id, Object,
    engine::{Edit, Engine},
    repository::{Error, Operation, Receipt, Repository, Selection, SelectionReceipt},
};
use std::sync::{Arc, Barrier};
fn select(repo: &mut Repository, value: &[u8], key: &str) -> SelectionReceipt {
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let blob = engine.blob(value.to_vec()).unwrap();
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
    repo.commit_source(&engine, root, repo.status().unwrap().generation, key)
        .unwrap()
}
fn values(s: Selection) -> (Id, Option<Id>) {
    (s.root, s.revision)
}
fn count(path: &std::path::Path) -> i64 {
    rusqlite::Connection::open(path.join(".avc/store.sqlite"))
        .unwrap()
        .query_row("SELECT count(*) FROM objects", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn amend_uses_selected_source_and_preserves_change_parents_and_predecessor() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let parent = repo
        .commit("Author", "base", 0, "base")
        .unwrap()
        .after
        .revision
        .unwrap();
    select(&mut repo, b"tip", "tip-source");
    let tip = repo
        .commit("Author", "tip", 2, "tip")
        .unwrap()
        .after
        .revision
        .unwrap();
    let original = repo.revision(tip).unwrap();
    let selected = select(&mut repo, b"selected dirty bytes", "working").after;
    let amended = repo.amend("New Author", "new message", 4, "amend").unwrap();
    assert_eq!(amended.operation, Operation::Amend);
    assert_eq!(amended.before, selected);
    let id = amended.after.revision.unwrap();
    let revision = repo.revision(id).unwrap();
    assert_ne!(id, tip);
    assert_eq!(revision.change, original.change);
    assert_eq!(revision.parents, vec![parent]);
    assert_eq!(revision.root, selected.root);
    assert_eq!(amended.after.root, selected.root);
    assert_eq!(
        (revision.author.as_str(), revision.message.as_str()),
        ("New Author", "new message")
    );
    assert_eq!(repo.revision(tip).unwrap(), original);
    assert_eq!(
        repo.log(None, 20)
            .unwrap()
            .iter()
            .map(|r| r.0)
            .collect::<Vec<_>>(),
        vec![id, parent]
    );
    let undone = repo.undo("amend", 5, "undo").unwrap();
    assert_eq!(
        undone.operation,
        Operation::Undo {
            target: "amend".into()
        }
    );
    assert_eq!(values(undone.after), values(selected));
    assert_eq!(undone.after.generation, 6);
    // Undo leaves the previously selected dirty source, not the old tip's source.
    assert_ne!(undone.after.root, original.root);
    let redone = repo.undo("undo", 6, "redo").unwrap();
    assert_eq!(values(redone.after), values(amended.after));
    assert_eq!(redone.after.generation, 7);
    assert_eq!(
        repo.amend("New Author", "new message", 4, "amend").unwrap(),
        amended
    );
    assert_eq!(repo.undo("amend", 5, "undo").unwrap(), undone);
    assert_eq!(repo.status().unwrap(), redone.after);
    let unchanged = repo
        .amend("New Author", "new message", 7, "unchanged")
        .unwrap();
    assert_ne!(unchanged.after.revision, amended.after.revision);
    assert_eq!(
        repo.revision(unchanged.after.revision.unwrap())
            .unwrap()
            .change,
        original.change
    );
    // An explicit amendment can leave an empty change relative to its parent.
    let mut empty = Engine::new();
    let empty_root = empty.empty().unwrap();
    repo.commit_source(&empty, empty_root, 8, "empty-source")
        .unwrap();
    let empty = repo
        .amend("Author", "empty result", 9, "empty-amend")
        .unwrap();
    assert_eq!(empty.after.root, repo.revision(parent).unwrap().root);
    assert_eq!(
        repo.revision(empty.after.revision.unwrap())
            .unwrap()
            .parents,
        vec![parent]
    );
    drop(repo);
    let repo = Repository::open(project.path()).unwrap();
    assert_eq!(repo.status().unwrap(), empty.after);
    assert_eq!(
        repo.receipt("undo").unwrap(),
        Some(Receipt::Selection(undone))
    );
    repo.load_root(tip).unwrap();
    repo.load_root(id).unwrap();
}

#[test]
fn undo_requires_the_exact_current_operation_and_retries_bind_target_identity() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let initial = repo.status().unwrap();
    assert!(repo.amend("Author", "unborn", 0, "amend").is_err());
    assert!(repo.undo("unknown", 0, "undo").is_err());
    assert_eq!(repo.status().unwrap(), initial);
    let first = repo.commit("Author", "one", 0, "one").unwrap();
    let second = repo.commit("Author", "two", 1, "two").unwrap();
    assert!(matches!(
        repo.undo("one", 2, "old-target"),
        Err(Error::Stale {
            expected: 1,
            actual: 2
        })
    ));
    assert!(matches!(
        repo.undo("two", 1, "stale"),
        Err(Error::Stale {
            expected: 1,
            actual: 2
        })
    ));
    let undo = repo.undo("two", 2, "undo").unwrap();
    assert_eq!(values(undo.after), values(first.after));
    assert!(matches!(
        repo.undo("one", 2, "undo"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert!(matches!(
        repo.undo("two", 3, "undo"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert!(matches!(
        repo.undo("", 2, "undo"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert!(matches!(
        repo.amend("Author", "two", 1, "two"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert!(matches!(
        repo.commit("Author", "one", 2, "undo"),
        Err(Error::RequestKeyReuse(_))
    ));
    let amended = repo.amend("Author", "amended", 3, "amend").unwrap();
    for (author, message) in [("", "amended"), ("Author", "changed")] {
        assert!(matches!(
            repo.amend(author, message, 3, "amend"),
            Err(Error::RequestKeyReuse(_))
        ));
    }
    assert!(matches!(
        repo.commit("Author", "amended", 3, "amend"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert_eq!(repo.undo("two", 2, "undo").unwrap(), undo);
    assert_eq!(repo.status().unwrap(), amended.after);
    repo.load_root(second.after.revision.unwrap()).unwrap();
}

#[test]
fn undo_source_and_first_commit_preserves_forward_generations_and_live_files() {
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("live"), b"not captured").unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let initial = repo.status().unwrap();
    let selected = select(&mut repo, b"data", "source");
    let authored = repo.commit("Author", "first", 1, "first").unwrap();
    let undo = repo.undo("first", 2, "undo-first").unwrap();
    assert_eq!(values(undo.after), values(selected.after));
    assert_eq!(undo.after.generation, 3);
    assert!(repo.log(None, 20).unwrap().is_empty());
    repo.load_root(authored.after.revision.unwrap()).unwrap();
    let select = select(&mut repo, b"other", "other");
    let undone_source = repo.undo("other", 4, "undo-source").unwrap();
    assert_eq!(values(undone_source.after), values(select.before));
    assert_eq!(
        std::fs::read(project.path().join("live")).unwrap(),
        b"not captured"
    );
    assert_eq!(values(selected.before), values(initial));
}

#[test]
fn concurrent_amend_and_undo_publish_once_or_reconcile_identical_requests() {
    for amend in [false, true] {
        for identical in [false, true] {
            let project = tempfile::tempdir().unwrap();
            let mut repo = Repository::init(project.path()).unwrap();
            let first = repo.commit("Author", "one", 0, "one").unwrap();
            drop(repo);
            let barrier = Arc::new(Barrier::new(2));
            let threads = (0..2)
                .map(|i| {
                    let mut repo = Repository::open(project.path()).unwrap();
                    let barrier = barrier.clone();
                    std::thread::spawn(move || {
                        barrier.wait();
                        let key = if identical || i == 0 { "a" } else { "b" };
                        if amend {
                            repo.amend("Author", "amended", 1, key)
                        } else {
                            repo.undo("one", 1, key)
                        }
                    })
                })
                .collect::<Vec<_>>();
            let results = threads
                .into_iter()
                .map(|t| t.join().unwrap())
                .collect::<Vec<_>>();
            if identical {
                assert_eq!(results[0].as_ref().unwrap(), results[1].as_ref().unwrap());
            } else {
                assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
                assert!(
                    results
                        .iter()
                        .any(|r| matches!(r, Err(Error::Stale { .. })))
                );
            }
            let repo = Repository::open(project.path()).unwrap();
            assert_eq!(repo.status().unwrap().generation, 2);
            if amend {
                assert_eq!(repo.log(None, 20).unwrap().len(), 1);
                assert_eq!(count(project.path()), 3);
            } else {
                assert!(repo.log(None, 20).unwrap().is_empty());
                assert_eq!(count(project.path()), 2);
            }
            repo.load_root(first.after.revision.unwrap()).unwrap();
        }
    }
}

#[test]
fn receipt_failure_rolls_back_amended_revision_and_undone_source_head_together() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    repo.commit("Author", "one", 0, "one").unwrap();
    select(&mut repo, b"working", "working");
    let current = repo.status().unwrap();
    let objects = count(project.path());
    let db = rusqlite::Connection::open(project.path().join(".avc/store.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER reject_edit BEFORE INSERT ON operations WHEN NEW.kind IN ('amend','undo') BEGIN SELECT RAISE(ABORT,'denied'); END;").unwrap();
    assert!(repo.amend("Author", "amended", 2, "amend").is_err());
    assert!(repo.undo("working", 2, "undo").is_err());
    assert_eq!(repo.status().unwrap(), current);
    assert_eq!(count(project.path()), objects);
    for key in ["amend", "undo"] {
        assert!(repo.receipt(key).unwrap().is_none());
    }
    db.execute_batch("DROP TRIGGER reject_edit").unwrap();
    repo.amend("Author", "amended", 2, "amend").unwrap();
}

#[test]
fn corrupt_selected_or_departed_source_cannot_gain_new_protection_via_edit_or_undo() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let old = select(&mut repo, b"old", "old").after;
    repo.commit("Author", "old", 1, "tip").unwrap();
    let selected = select(&mut repo, b"new", "new").after;
    let db = rusqlite::Connection::open(project.path().join(".avc/store.sqlite")).unwrap();
    db.execute_batch("DROP TRIGGER objects_no_update").unwrap();
    for root in [old.root, selected.root] {
        let engine = repo.load_root(root).unwrap();
        let Entry::File { blob, .. } = engine.read(root, &[b"file".to_vec()]).unwrap() else {
            panic!()
        };
        assert!(matches!(engine.object(blob).unwrap(), Object::Blob(_)));
        db.execute(
            "UPDATE objects SET payload=?1 WHERE id=?2",
            rusqlite::params![b"bad".as_slice(), blob.to_bytes().unwrap().as_slice()],
        )
        .unwrap();
    }
    assert!(repo.amend("Author", "refuse", 3, "amend").is_err());
    assert!(repo.undo("new", 3, "undo").is_err());
    assert_eq!(repo.status().unwrap(), selected);
    for key in ["amend", "undo"] {
        assert!(repo.receipt(key).unwrap().is_none());
    }
}

#[test]
fn selected_history_export_preserves_rewrite_but_does_not_promise_private_undo_roots() {
    let original = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(original.path()).unwrap();
    select(&mut repo, b"before", "before");
    let old = repo
        .commit("Author", "old", 1, "old")
        .unwrap()
        .after
        .revision
        .unwrap();
    select(&mut repo, b"after", "after");
    let amended = repo.amend("Author", "new", 3, "amend").unwrap();
    let mut bundle = Vec::new();
    repo.export_history(&mut bundle).unwrap();
    let fresh = tempfile::tempdir().unwrap();
    let mut restored = Repository::init(fresh.path()).unwrap();
    restored
        .import_history(bundle.as_slice(), "restore")
        .unwrap();
    assert_eq!(
        restored.revision(amended.after.revision.unwrap()).unwrap(),
        repo.revision(amended.after.revision.unwrap()).unwrap()
    );
    assert!(matches!(
        restored.revision(old),
        Err(Error::MissingObject(_))
    ));
    repo.load_root(old).unwrap();
    assert!(restored.receipt("amend").unwrap().is_none());
    let undo = restored.undo("restore", 1, "undo-restore").unwrap();
    assert!(undo.after.revision.is_none());
    assert_eq!(undo.after.generation, 2);
    restored.load_root(amended.after.revision.unwrap()).unwrap();
}

#[test]
fn amend_checks_the_actual_new_parent_metadata_before_publication() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let parent = repo
        .commit("Author", "base", 0, "base")
        .unwrap()
        .after
        .revision
        .unwrap();
    repo.commit("Author", "tip", 1, "tip").unwrap();
    let before = repo.status().unwrap();
    let mut damaged = Object::Revision(repo.revision(parent).unwrap())
        .encode()
        .unwrap();
    *damaged.last_mut().unwrap() ^= 1;
    let db = rusqlite::Connection::open(project.path().join(".avc/store.sqlite")).unwrap();
    db.execute_batch("DROP TRIGGER objects_no_update").unwrap();
    db.execute(
        "UPDATE objects SET payload=?1 WHERE id=?2",
        rusqlite::params![damaged, parent.to_bytes().unwrap().as_slice()],
    )
    .unwrap();
    assert!(repo.amend("Author", "rewrite", 2, "amend").is_err());
    assert_eq!(repo.status().unwrap(), before);
    assert!(repo.receipt("amend").unwrap().is_none());
}
