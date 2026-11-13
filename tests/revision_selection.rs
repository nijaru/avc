use avc::{
    Conflict, Entry, Id, Object, REVISION_KIND,
    engine::{Edit, Engine},
    repository::{
        CaptureScope, Error, Operation, Receipt, RefName, Repository, Selection, SelectionReceipt,
    },
};
use rusqlite::Connection;
use std::{
    path::Path,
    sync::{Arc, Barrier},
};

fn database(project: &Path) -> Connection {
    // SQLite owns database/WAL/SHM descriptors, even for damage injection.
    Connection::open(project.join(".avc/store.sqlite")).unwrap()
}

fn objects(project: &Path) -> Vec<(Vec<u8>, i64, Vec<u8>)> {
    database(project)
        .prepare("SELECT id, length, payload FROM objects ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn operation_count(project: &Path) -> i64 {
    database(project)
        .query_row("SELECT count(*) FROM operations", [], |r| r.get(0))
        .unwrap()
}

fn corrupt(project: &Path, id: Id) {
    let db = database(project);
    db.execute_batch("DROP TRIGGER IF EXISTS objects_no_update")
        .unwrap();
    assert_eq!(
        db.execute(
            "UPDATE objects SET payload = zeroblob(length) WHERE id = ?1",
            [id.to_bytes().unwrap().as_slice()],
        )
        .unwrap(),
        1
    );
}

fn remove(project: &Path, id: Id) {
    let db = database(project);
    db.pragma_update(None, "foreign_keys", false).unwrap();
    db.execute_batch("DROP TRIGGER IF EXISTS objects_no_delete")
        .unwrap();
    assert_eq!(
        db.execute(
            "DELETE FROM objects WHERE id = ?1",
            [id.to_bytes().unwrap().as_slice()]
        )
        .unwrap(),
        1
    );
}

fn source(bytes: &[u8]) -> (Engine, Id, Id) {
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
    (engine, root, blob)
}

fn edit(repo: &mut Repository, bytes: &[u8], key: &str) -> Id {
    let (engine, root, blob) = source(bytes);
    repo.commit_source(&engine, root, repo.status().unwrap().generation, key)
        .unwrap();
    blob
}

fn commit(repo: &mut Repository, key: &str) -> Id {
    repo.commit(
        "Contract author",
        key,
        repo.status().unwrap().generation,
        key,
    )
    .unwrap()
    .after
    .revision
    .unwrap()
}

fn two_revisions(repo: &mut Repository) -> (Id, Id, Id, Id) {
    let a_blob = edit(repo, b"A", "source-a");
    let a = commit(repo, "commit-a");
    let b_blob = edit(repo, b"B", "source-b");
    let b = commit(repo, "commit-b");
    (a, b, a_blob, b_blob)
}

fn name(text: &str) -> RefName {
    text.parse().unwrap()
}

fn assert_rejected(repo: &mut Repository, project: &Path, target: Id, key: &str) {
    let before = repo.status().unwrap();
    let retained = objects(project);
    let operations = operation_count(project);
    assert!(
        repo.select_revision(target, before.generation, key)
            .is_err(),
        "accepted {key}"
    );
    assert_eq!(repo.status().unwrap(), before);
    assert_eq!(objects(project), retained);
    assert_eq!(operation_count(project), operations);
    assert!(repo.receipt(key).unwrap().is_none());
}

#[test]
fn exact_selection_roundtrip_branches_from_saved_revision_without_advancing_names() {
    let project = tempfile::tempdir().unwrap();
    let p = project.path();
    let mut repo = Repository::init(p).unwrap();
    let (a, b, _, _) = two_revisions(&mut repo);
    let a_metadata = repo.revision(a).unwrap();
    let saved = repo.set_reference(&name("saved"), a, 0, "save-a").unwrap();
    let tip = repo.set_reference(&name("tip"), b, 0, "save-b").unwrap();
    let exact = repo.reference(&name("saved")).unwrap().unwrap().revision;
    // A name is merely a way to discover an exact ID; its later move is irrelevant.
    let moved = repo
        .set_reference(&name("saved"), b, saved.after.generation, "move-name")
        .unwrap();
    let before = repo.status().unwrap();
    let retained = objects(p);
    let receipt: SelectionReceipt = repo
        .select_revision(exact, before.generation, "select-a")
        .unwrap();
    assert_eq!(receipt.request_key, "select-a");
    assert_eq!(receipt.operation, Operation::Select { revision: a });
    assert_eq!(receipt.before, before);
    assert_eq!(
        receipt.after,
        Selection {
            root: a_metadata.root,
            revision: Some(a),
            generation: before.generation + 1
        }
    );
    assert_eq!(repo.status().unwrap(), receipt.after);
    assert_eq!(
        objects(p),
        retained,
        "selection admits no objects or authored metadata"
    );
    assert_eq!(repo.revision(a).unwrap(), a_metadata);
    assert_eq!(repo.reference(&name("saved")).unwrap(), Some(moved.after));
    assert_eq!(repo.reference(&name("tip")).unwrap(), Some(tip.after));
    drop(repo);
    let mut repo = Repository::open(p).unwrap();
    assert_eq!(
        repo.receipt("select-a").unwrap(),
        Some(Receipt::Selection(receipt.clone()))
    );
    let restored = repo
        .undo("select-a", receipt.after.generation, "undo-select")
        .unwrap();
    assert_eq!(
        (restored.after.root, restored.after.revision),
        (before.root, Some(b))
    );
    assert_eq!(restored.after.generation, receipt.after.generation + 1);
    let redone = repo
        .undo("undo-select", restored.after.generation, "undo-undo")
        .unwrap();
    assert_eq!(
        (redone.after.root, redone.after.revision),
        (a_metadata.root, Some(a))
    );
    assert_eq!(redone.after.generation, restored.after.generation + 1);
    assert_eq!(objects(p), retained);
    edit(&mut repo, b"branch C", "source-c");
    let c = commit(&mut repo, "commit-c");
    assert_eq!(repo.revision(c).unwrap().parents, vec![a]);
    assert_eq!(repo.reference(&name("saved")).unwrap(), Some(moved.after));
    assert_eq!(repo.reference(&name("tip")).unwrap(), Some(tip.after));
    let now = repo.status().unwrap();
    assert!(repo.undo("select-a", now.generation, "too-late").is_err());
    assert!(repo.receipt("too-late").unwrap().is_none());
    assert_eq!(repo.status().unwrap(), now);
}

#[test]
fn dirty_authored_and_unborn_source_refuse_but_logically_clean_source_is_allowed() {
    for authored in [false, true] {
        let project = tempfile::tempdir().unwrap();
        let mut repo = Repository::init(project.path()).unwrap();
        let target = commit(&mut repo, "empty-commit");
        if !authored {
            repo.undo("empty-commit", 1, "unborn").unwrap();
        }
        edit(&mut repo, b"selected but not committed", "dirty");
        let before = repo.status().unwrap();
        let error = repo
            .select_revision(target, before.generation, "select")
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("selected-but-uncommitted source"),
            "{error}"
        );
        assert_rejected(&mut repo, project.path(), target, "select");
        assert_eq!(repo.status().unwrap(), before);
    }
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let (a, b, _, _) = two_revisions(&mut repo);
    let (engine, b_root, _) = source(b"B");
    let before = repo.status().unwrap();
    repo.commit_source(&engine, b_root, before.generation, "same-logical-source")
        .unwrap();
    assert_eq!(repo.status().unwrap().revision, Some(b));
    repo.select_revision(a, before.generation + 1, "clean-select")
        .unwrap();
}

#[test]
fn canonical_empty_unborn_after_undo_first_commit_can_select_an_existing_revision() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let empty = repo.status().unwrap();
    let target = commit(&mut repo, "first");
    let undone = repo.undo("first", 1, "undo-first").unwrap();
    assert_eq!(undone.after.root, empty.root);
    assert_eq!(undone.after.revision, None);
    let selected = repo
        .select_revision(target, undone.after.generation, "select-existing")
        .unwrap();
    assert_eq!(selected.before, undone.after);
    assert_eq!(selected.after.root, empty.root);
    assert_eq!(selected.after.revision, Some(target));
    assert_eq!(selected.after.generation, 3);
}

#[test]
fn same_target_advances_once_and_exact_retry_precedes_staleness_dirt_and_target_damage() {
    let project = tempfile::tempdir().unwrap();
    let p = project.path();
    let mut repo = Repository::init(p).unwrap();
    let (a, b, _, b_blob) = two_revisions(&mut repo);
    let before = repo.status().unwrap();
    let retained = objects(p);
    let count = operation_count(p);
    let selected = repo
        .select_revision(b, before.generation, "same-target")
        .unwrap();
    assert_eq!(
        selected.after,
        Selection {
            generation: before.generation + 1,
            ..before
        }
    );
    assert_eq!(objects(p), retained);
    assert_eq!(operation_count(p), count + 1);
    let later = repo
        .select_revision(a, selected.after.generation, "later-select")
        .unwrap();
    repo.set_reference(&name("later"), a, 0, "later-ref")
        .unwrap();
    edit(&mut repo, b"later dirty source", "later-dirty");
    let current = repo.status().unwrap();
    assert!(current.generation > later.after.generation);
    corrupt(p, b_blob);
    corrupt(p, b);
    drop(repo);
    let mut repo = Repository::open(p).unwrap();
    let retained = objects(p);
    let count = operation_count(p);
    assert_eq!(
        repo.select_revision(b, before.generation, "same-target")
            .unwrap(),
        selected
    );
    assert_eq!(
        repo.receipt("same-target").unwrap(),
        Some(Receipt::Selection(selected))
    );
    for (target, generation) in [(a, before.generation), (b, before.generation + 1)] {
        assert!(matches!(
            repo.select_revision(target, generation, "same-target"),
            Err(Error::RequestKeyReuse(_))
        ));
    }
    assert_eq!(repo.status().unwrap(), current);
    assert_eq!(objects(p), retained);
    assert_eq!(operation_count(p), count);
}

#[test]
fn fresh_selection_verifies_target_source_not_historical_payloads_or_departed_source() {
    // Both parent metadata and blobs outside the current target Source are only
    // historical ownership, not part of this operation's health certificate.
    for damage_metadata in [false, true] {
        let project = tempfile::tempdir().unwrap();
        let mut repo = Repository::init(project.path()).unwrap();
        let (a, b, a_blob, _) = two_revisions(&mut repo);
        corrupt(project.path(), if damage_metadata { a } else { a_blob });
        assert!(repo.load_root(b).is_err());
        let before = repo.status().unwrap();
        let selected = repo
            .select_revision(b, before.generation, "healthy-current-source")
            .unwrap();
        assert_eq!(selected.after.revision, Some(b));
        assert_eq!(selected.after.generation, before.generation + 1);
    }
    for damage_root in [false, true] {
        let project = tempfile::tempdir().unwrap();
        let mut repo = Repository::init(project.path()).unwrap();
        let (a, b, _, b_blob) = two_revisions(&mut repo);
        let b_root = repo.revision(b).unwrap().root;
        corrupt(project.path(), if damage_root { b_root } else { b_blob });
        assert!(repo.load_root(b_root).is_err());
        let before = repo.status().unwrap();
        let selected = repo
            .select_revision(a, before.generation, "leave-damaged-source")
            .unwrap();
        assert_eq!(
            selected.before, before,
            "ACK retains metadata, not a departed-data certificate"
        );
        assert_eq!(selected.after.root, repo.revision(a).unwrap().root);
        assert_eq!(selected.after.revision, Some(a));
    }
}

#[test]
fn bad_target_kind_missing_revision_dependencies_and_payload_publish_nothing() {
    for damage in [
        "missing-parent",
        "missing-root",
        "missing-blob",
        "root-payload",
        "blob-payload",
        "revision-payload",
    ] {
        let project = tempfile::tempdir().unwrap();
        let p = project.path();
        let mut repo = Repository::init(p).unwrap();
        let (a, b, _, b_blob) = two_revisions(&mut repo);
        let root = repo.revision(b).unwrap().root;
        assert_rejected(&mut repo, p, root, "wrong-map-kind");
        assert_rejected(&mut repo, p, b_blob, "wrong-blob-kind");
        assert_rejected(
            &mut repo,
            p,
            Id {
                kind: REVISION_KIND,
                digest: [0; 32],
            },
            "unknown-revision",
        );
        match damage {
            "missing-parent" => remove(p, a),
            "missing-root" => remove(p, root),
            "missing-blob" => remove(p, b_blob),
            "root-payload" => corrupt(p, root),
            "blob-payload" => corrupt(p, b_blob),
            "revision-payload" => corrupt(p, b),
            _ => unreachable!(),
        }
        assert_rejected(&mut repo, p, b, damage);
    }
}

#[test]
fn hash_valid_but_noncanonical_revision_metadata_is_not_a_selection_target() {
    let project = tempfile::tempdir().unwrap();
    let p = project.path();
    let mut repo = Repository::init(p).unwrap();
    let (_, b, _, _) = two_revisions(&mut repo);
    let mut payload = Object::Revision(repo.revision(b).unwrap())
        .encode()
        .unwrap();
    payload.push(0); // Valid ID/hash, invalid trailing canonical revision bytes.
    let id = Id::hash(REVISION_KIND, &payload).unwrap();
    database(p)
        .execute(
            "INSERT INTO objects(id, length, payload) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                id.to_bytes().unwrap().as_slice(),
                i64::try_from(payload.len()).unwrap(),
                payload
            ],
        )
        .unwrap();
    assert_rejected(&mut repo, p, id, "noncanonical-metadata");
}

#[test]
fn global_request_keys_do_not_cross_kinds_and_ref_generations_do_not_pin_source() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let (a, b, _, _) = two_revisions(&mut repo);
    let before = repo.status().unwrap();
    let reference = repo.set_reference(&name("saved"), b, 0, "ref-key").unwrap();
    // Ref writes and reads do not consume the expected Source generation.
    assert_eq!(repo.status().unwrap(), before);
    assert_eq!(
        repo.reference(&name("saved")).unwrap(),
        Some(reference.after)
    );
    repo.references(None, 20).unwrap();
    let selected = repo
        .select_revision(a, before.generation, "select-key")
        .unwrap();
    let moved = repo
        .set_reference(&name("saved"), a, 1, "ref-after-select")
        .unwrap();
    assert_eq!(moved.before, Some(reference.after));
    for key in ["source-a", "commit-a", "ref-key"] {
        assert!(matches!(
            repo.select_revision(a, selected.after.generation, key),
            Err(Error::RequestKeyReuse(_))
        ));
    }
    let (engine, root, _) = source(b"unused");
    // This scope is deliberately unavailable: cross-kind replay must refuse
    // before observing even a single live path.
    let scope = CaptureScope::new(
        &project.path().join("does-not-exist"),
        vec![vec![b"file".to_vec()]],
    )
    .unwrap();
    assert!(matches!(
        repo.capture(&scope, selected.after.generation, "select-key"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert!(matches!(
        repo.commit_source(&engine, root, selected.after.generation, "select-key"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert!(matches!(
        repo.commit("Other", "unused", selected.after.generation, "select-key"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert!(matches!(
        repo.set_reference(&name("new"), a, 0, "select-key"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert!(matches!(
        repo.undo("select-key", selected.after.generation, "select-key"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert!(
        repo.undo("ref-key", selected.after.generation, "cannot-undo-ref")
            .is_err()
    );
    assert!(repo.receipt("cannot-undo-ref").unwrap().is_none());
    assert_eq!(repo.status().unwrap(), selected.after);
    let undone = repo
        .undo("select-key", selected.after.generation, "undo-select")
        .unwrap();
    assert_eq!(undone.after.revision, Some(b));
    assert_eq!(repo.reference(&name("saved")).unwrap(), Some(moved.after));
    assert!(matches!(
        repo.select_revision(a, undone.after.generation, "undo-select"),
        Err(Error::RequestKeyReuse(_))
    ));
}

#[cfg(unix)]
#[test]
fn selection_cannot_reuse_a_capture_observation_key() {
    let project = tempfile::tempdir().unwrap();
    let p = project.path();
    let mut repo = Repository::init(p).unwrap();
    let (a, _, _, _) = two_revisions(&mut repo);
    std::fs::write(p.join("file"), b"B").unwrap();
    let scope = CaptureScope::new(p, vec![vec![b"file".to_vec()]]).unwrap();
    let captured = repo.capture(&scope, 4, "capture-key").unwrap();
    assert!(matches!(
        repo.select_revision(a, captured.after.generation, "capture-key"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert_eq!(repo.status().unwrap(), captured.after);
    assert_eq!(
        repo.receipt("capture-key").unwrap(),
        Some(Receipt::Selection(captured))
    );
}

#[test]
fn concurrent_identical_selects_share_receipt_and_different_targets_have_one_cas_winner() {
    for identical in [false, true] {
        let project = tempfile::tempdir().unwrap();
        let mut repo = Repository::init(project.path()).unwrap();
        let (a, b, _, _) = two_revisions(&mut repo);
        let before = repo.status().unwrap();
        let count = operation_count(project.path());
        drop(repo);
        let barrier = Arc::new(Barrier::new(2));
        let workers = (0..2)
            .map(|i| {
                // Open BEFORE the barrier: connection startup fences must not serialize
                // away the race that the shared publication transaction must arbitrate.
                let mut repo = Repository::open(project.path()).unwrap();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    repo.select_revision(
                        if identical || i == 0 { a } else { b },
                        before.generation,
                        if identical || i == 0 { "one" } else { "two" },
                    )
                })
            })
            .collect::<Vec<_>>();
        let results = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        if identical {
            assert_eq!(results[0].as_ref().unwrap(), results[1].as_ref().unwrap());
        } else {
            assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
            assert_eq!(results.iter().filter(|r| matches!(r, Err(Error::Stale { expected, actual }) if *expected == before.generation && *actual == before.generation + 1)).count(), 1);
        }
        let repo = Repository::open(project.path()).unwrap();
        assert_eq!(repo.status().unwrap().generation, before.generation + 1);
        assert_eq!(operation_count(project.path()), count + 1);
    }
}

#[test]
fn source_and_reference_cas_can_both_win_concurrently() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let (a, b, _, _) = two_revisions(&mut repo);
    let before = repo.status().unwrap();
    repo.set_reference(&name("saved"), b, 0, "ref-birth")
        .unwrap();
    let mut selector = Repository::open(project.path()).unwrap();
    let mut namer = Repository::open(project.path()).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let other = Arc::clone(&barrier);
    let selection = std::thread::spawn(move || {
        other.wait();
        selector.select_revision(a, before.generation, "select")
    });
    let reference = std::thread::spawn(move || {
        barrier.wait();
        namer.set_reference(&name("saved"), a, 1, "ref-move")
    });
    let selected = selection.join().unwrap().unwrap();
    let named = reference.join().unwrap().unwrap();
    assert_eq!(repo.status().unwrap(), selected.after);
    assert_eq!(repo.reference(&name("saved")).unwrap(), Some(named.after));
    assert_eq!(selected.after.generation, before.generation + 1);
    assert_eq!(named.after.generation, 2);
}

#[test]
fn authored_source_root_conflicts_are_selectable_without_fabricating_resolution() {
    let project = tempfile::tempdir().unwrap();
    let p = project.path();
    let mut repo = Repository::init(p).unwrap();
    let ordinary = commit(&mut repo, "ordinary");
    let (mut engine, left, _) = source(b"left");
    let (right_engine, right, right_blob) = source(b"right");
    let empty = engine.empty().unwrap();
    let mut records = engine
        .objects()
        .chain(right_engine.objects())
        .map(|(id, object)| (id, object.encode().unwrap()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut positive = vec![(Entry::Directory(left), 1), (Entry::Directory(right), 1)];
    positive.sort_by_key(|(entry, _)| entry.encode().unwrap());
    let conflict = Object::Conflict(Conflict {
        region: 1,
        positive,
        negative: vec![(Entry::Directory(empty), 1)],
    });
    let root = conflict.id().unwrap();
    records.insert(root, conflict.encode().unwrap());
    let mut imported = Engine::new();
    imported.import(records, root, Default::default()).unwrap();
    repo.commit_source(&imported, root, 1, "conflict-source")
        .unwrap();
    let authored = commit(&mut repo, "conflict-commit");
    repo.select_revision(ordinary, 3, "leave-conflict").unwrap();
    let retained = objects(p);
    let selected = repo
        .select_revision(authored, 4, "select-conflict")
        .unwrap();
    assert_eq!(selected.after.root, root);
    assert_eq!(selected.after.revision, Some(authored));
    assert_eq!(objects(p), retained);
    assert_eq!(
        repo.load_root(root).unwrap().object(root).unwrap(),
        &conflict
    );
    // Clean means exact root equality, not conflict-free source.
    repo.select_revision(ordinary, 5, "leave-again").unwrap();
    corrupt(p, right_blob);
    assert_rejected(&mut repo, p, authored, "conflict-term-payload-damaged");
}

#[test]
fn selection_does_not_read_or_write_live_files_or_interpret_git_administration() {
    let project = tempfile::tempdir().unwrap();
    let p = project.path();
    let mut repo = Repository::init(p).unwrap();
    let (a, _, _, _) = two_revisions(&mut repo);
    std::fs::write(p.join("file"), b"live dirty bytes, not A or B").unwrap();
    std::fs::write(p.join(".git"), b"not a Git repository or usable gitdir").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(p.join("file"), std::fs::Permissions::from_mode(0o0)).unwrap();
        std::os::unix::fs::symlink(
            "/nonexistent-avc-selection-target",
            p.join("unreadable-link"),
        )
        .unwrap();
    }
    let retained = objects(p);
    let before = repo.status().unwrap();
    let selected = repo
        .select_revision(a, before.generation, "select-without-checkout")
        .unwrap();
    repo.undo(
        "select-without-checkout",
        selected.after.generation,
        "undo-without-checkout",
    )
    .unwrap();
    assert_eq!(objects(p), retained);
    let db = database(p);
    for table in ["git_origins", "git_exports"] {
        assert_eq!(
            db.query_row::<i64, _, _>(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                .unwrap(),
            0
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(p.join("file"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0
        );
        assert_eq!(
            std::fs::read_link(p.join("unreadable-link")).unwrap(),
            Path::new("/nonexistent-avc-selection-target")
        );
        std::fs::set_permissions(p.join("file"), std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert_eq!(
        std::fs::read(p.join("file")).unwrap(),
        b"live dirty bytes, not A or B"
    );
    assert_eq!(
        std::fs::read(p.join(".git")).unwrap(),
        b"not a Git repository or usable gitdir"
    );
}

#[test]
fn invalid_generations_and_keys_do_not_publish_selection() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let target = commit(&mut repo, "target");
    let before = repo.status().unwrap();
    for generation in [0, 2, i64::MAX as u64, i64::MAX as u64 + 1, u64::MAX] {
        assert!(
            repo.select_revision(target, generation, "bad-generation")
                .is_err()
        );
        assert!(repo.receipt("bad-generation").unwrap().is_none());
    }
    for key in [String::new(), "x".repeat(1025)] {
        assert!(
            repo.select_revision(target, before.generation, &key)
                .is_err()
        );
    }
    assert_eq!(repo.status().unwrap(), before);
    let key = "x".repeat(1024);
    repo.select_revision(target, before.generation, &key)
        .unwrap();
    assert!(repo.receipt(&key).unwrap().is_some());
}

#[test]
fn retained_git_anchor_selection_needs_no_git_or_origin_lookup_and_keeps_amend_protection() {
    // Independent empty-tree Git fixture, hashed with Python hashlib rather
    // than created by a Git process. Import the literal portable history wire.
    let project = tempfile::tempdir().unwrap();
    let p = project.path();
    let mut repo = Repository::init(p).unwrap();
    let mut engine = Engine::new();
    let root = engine.empty().unwrap();
    let oid: avc::git::GitOid = "62cbbd4e5017bd8aa6cc6286049c62b19d305d0c".parse().unwrap();
    let raw = b"tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\nauthor Fixture <fixture@example.test> 1700000000 +0000\ncommitter Fixture <fixture@example.test> 1700000000 +0000\n\nanchor\n";
    let mut identity = blake3::Hasher::new();
    identity.update(b"AVC-GIT-ANCHOR\0v1\0");
    identity.update(&[1, 20]);
    identity.update(&oid.to_bytes()[1..]);
    let mut change = [0; 16];
    change.copy_from_slice(&identity.finalize().as_bytes()[..16]);
    let revision = Object::Revision(avc::Revision {
        root,
        parents: vec![],
        change,
        author: "Fixture <fixture@example.test>".into(),
        message: "anchor\n".into(),
        time_ns: 1_700_000_000_000_000_000,
        timezone_minutes: 0,
    });
    let anchor = revision.id().unwrap();
    let mut records = engine
        .objects()
        .map(|(id, object)| (id, object.encode().unwrap()))
        .collect::<std::collections::BTreeMap<_, _>>();
    records.insert(anchor, revision.encode().unwrap());
    let mut wire = b"AVCHIS02".to_vec();
    wire.extend(root.to_bytes().unwrap());
    wire.push(1);
    wire.extend(anchor.to_bytes().unwrap());
    wire.extend(u64::try_from(records.len()).unwrap().to_le_bytes());
    for (id, payload) in records {
        wire.extend(id.to_bytes().unwrap());
        wire.extend(u64::try_from(payload.len()).unwrap().to_le_bytes());
        wire.extend(payload);
    }
    wire.extend(1_u64.to_le_bytes());
    wire.extend(anchor.to_bytes().unwrap());
    wire.extend(oid.to_bytes());
    wire.extend(u64::try_from(raw.len()).unwrap().to_le_bytes());
    wire.extend(raw);
    wire.extend(b"AVCHEND2");
    wire.extend(*blake3::hash(&wire).as_bytes());
    repo.import_history(wire.as_slice(), "portable-anchor")
        .unwrap();
    let origin = repo.git_origin(anchor).unwrap().unwrap();
    let child = commit(&mut repo, "native-child");
    let retained = objects(p);
    repo.select_revision(anchor, 2, "select-anchor").unwrap();
    assert_eq!(repo.git_origin(anchor).unwrap(), Some(origin));
    assert_eq!(objects(p), retained);
    assert!(
        repo.amend("Author", "must not rewrite anchor", 3, "amend-anchor")
            .is_err()
    );
    assert!(repo.receipt("amend-anchor").unwrap().is_none());
    let mut exported = Vec::new();
    repo.export_history(&mut exported).unwrap();
    assert_eq!(
        exported, wire,
        "selection receipts do not change portable origin history"
    );
    repo.select_revision(child, 3, "select-child").unwrap();
    // Selection is not an origin-health certificate. Retained import custody
    // still protects amend, even when its actual origin row has been damaged.
    let db = database(p);
    db.pragma_update(None, "foreign_keys", false).unwrap();
    db.execute_batch("DROP TRIGGER git_origins_no_delete; DELETE FROM git_origins;")
        .unwrap();
    drop(db);
    let selected = repo
        .select_revision(anchor, 4, "anchor-without-origin")
        .unwrap();
    assert_eq!(selected.after.revision, Some(anchor));
    assert_eq!(objects(p), retained);
    assert!(
        repo.amend(
            "Author",
            "still must not rewrite anchor",
            5,
            "amend-damaged-anchor"
        )
        .is_err()
    );
    assert!(repo.receipt("amend-damaged-anchor").unwrap().is_none());
    assert_eq!(repo.status().unwrap(), selected.after);
    assert!(!p.join(".git").exists());
    assert_eq!(
        database(p)
            .query_row::<i64, _, _>("SELECT count(*) FROM git_exports", [], |r| r.get(0))
            .unwrap(),
        0
    );
}
