use avc::{
    Conflict, Entry, Id, Node, Object, Revision,
    engine::{Edit, Engine, Limits},
    repository::{Error, MAX_OBJECT_BYTES, Repository},
};
use rusqlite::Connection;
use std::{
    collections::BTreeSet,
    fs,
    io::{self, Cursor, Write},
    path::Path,
    sync::{Arc, Barrier},
    thread,
};

fn file(blob: Id) -> Entry {
    Entry::File {
        blob,
        executable: false,
    }
}
/// Fixed payload shared with the storage fault and historical control fixtures.
const FIXTURE_PAYLOAD_BYTES: usize = 8192;
fn path(name: &[u8]) -> Vec<Vec<u8>> {
    vec![name.to_vec()]
}
fn put(engine: &mut Engine, root: Id, name: &[u8], value: Entry) -> Id {
    let expected = engine.read(root, &path(name)).unwrap();
    engine
        .edit(
            root,
            &[Edit {
                path: path(name),
                expected,
                value,
            }],
        )
        .unwrap()
}
fn source(bytes: Vec<u8>) -> (Engine, Id, Id) {
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let blob = engine.blob(bytes).unwrap();
    let root = put(&mut engine, empty, b"file", file(blob));
    (engine, root, blob)
}
fn database(project: &Path) -> Connection {
    Connection::open(project.join(".avc/store.sqlite")).unwrap()
}
fn object_ids(project: &Path) -> BTreeSet<Id> {
    database(project)
        .prepare("SELECT id FROM objects")
        .unwrap()
        .query_map([], |r| r.get::<_, Vec<u8>>(0))
        .unwrap()
        .map(|r| Id::from_bytes(&r.unwrap()).unwrap())
        .collect()
}
#[test]
fn init_is_an_empty_source_pin_and_open_never_creates_a_database() {
    let project = tempfile::tempdir().unwrap();
    assert!(Repository::open(project.path()).is_err());
    assert!(!project.path().join(".avc").exists());
    let repository = Repository::init(project.path()).unwrap();
    let (initial, engine) = repository.load().unwrap();
    assert_eq!(initial.generation, 0);
    assert_eq!(engine.object_count(), 1);
    assert_eq!(
        engine.object(initial.root).unwrap(),
        &Object::Map(Node::Leaf(vec![]))
    );
    assert!(engine.list(initial.root, &[]).unwrap().is_empty());
    let db = database(project.path());
    assert_eq!(
        db.pragma_query_value::<String, _>(None, "journal_mode", |r| r.get(0))
            .unwrap(),
        "wal"
    );
    assert_eq!(
        db.query_row::<i64, _, _>("SELECT count(*) FROM operations", [], |r| r.get(0))
            .unwrap(),
        0
    );
    assert!(matches!(
        Repository::init(project.path()),
        Err(Error::AlreadyExists(_))
    ));
    assert_eq!(repository.status().unwrap(), initial);
    drop(db);
    drop(repository);
    assert_eq!(
        Repository::open(project.path()).unwrap().status().unwrap(),
        initial
    );

    let missing_db = tempfile::tempdir().unwrap();
    fs::create_dir(missing_db.path().join(".avc")).unwrap();
    assert!(Repository::open(missing_db.path()).is_err());
    assert!(!missing_db.path().join(".avc/store.sqlite").exists());
}

#[test]
fn unsupported_physical_schema_refuses_without_migration_or_data_changes() {
    let project = tempfile::tempdir().unwrap();
    drop(Repository::init(project.path()).unwrap());
    let db = database(project.path());
    assert_eq!(
        db.pragma_query_value::<i64, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        4
    );
    db.pragma_update(None, "user_version", 3).unwrap();
    drop(db);
    // Retain a witness in the obsolete raw-file layout as well as the database.
    let objects = project.path().join(".avc/objects/sealed");
    fs::create_dir_all(&objects).unwrap();
    let witness = objects.join("old-object");
    fs::write(&witness, b"preserve old payload").unwrap();
    let database_path = project.path().join(".avc/store.sqlite");
    let before = fs::read(&database_path).unwrap();
    let error = match Repository::open(project.path()) {
        Ok(_) => panic!("old schema accepted"),
        Err(error) => error,
    };
    assert!(
        matches!(&error, Error::Invalid(message) if message.contains("unsupported physical schema 3"))
    );
    assert_eq!(fs::read(&database_path).unwrap(), before);
    assert_eq!(fs::read(witness).unwrap(), b"preserve old payload");
    assert!(matches!(
        Repository::init(project.path()),
        Err(Error::AlreadyExists(_))
    ));
}

#[test]
fn init_refuses_existing_files_and_legacy_directories_without_mutation() {
    for directory in [false, true] {
        let project = tempfile::tempdir().unwrap();
        let admin = project.path().join(".avc");
        let witness = if directory {
            fs::create_dir(&admin).unwrap();
            admin.join("legacy")
        } else {
            admin.clone()
        };
        fs::write(&witness, b"do not touch").unwrap();
        assert!(matches!(
            Repository::init(project.path()),
            Err(Error::AlreadyExists(_))
        ));
        assert_eq!(fs::read(&witness).unwrap(), b"do not touch");
        if directory {
            assert_eq!(fs::read_dir(&admin).unwrap().count(), 1);
        }
    }
}

#[cfg(unix)]
#[test]
fn init_refuses_symlinks_including_dangling_without_touching_targets() {
    use std::os::unix::fs::symlink;
    for kind in ["missing", "file", "directory"] {
        let project = tempfile::tempdir().unwrap();
        let target = project.path().join("target");
        if kind == "directory" {
            fs::create_dir(&target).unwrap();
        }
        if kind == "file" {
            fs::write(&target, b"old bytes").unwrap();
        }
        let admin = project.path().join(".avc");
        symlink(&target, &admin).unwrap();
        assert!(matches!(
            Repository::init(project.path()),
            Err(Error::AlreadyExists(_))
        ));
        assert_eq!(fs::read_link(&admin).unwrap(), target);
        if kind == "missing" {
            assert!(!target.exists());
        }
        if kind == "file" {
            assert_eq!(fs::read(&target).unwrap(), b"old bytes");
        }
        if kind == "directory" {
            assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
        }
    }
}

#[test]
fn reopen_restores_exact_closure_and_receipts_keep_departure_source() {
    let project = tempfile::tempdir().unwrap();
    let mut repository = Repository::init(project.path()).unwrap();
    let initial = repository.status().unwrap();
    let (mut engine, first, large) = source(vec![3; FIXTURE_PAYLOAD_BYTES]);
    let orphan = engine.blob(b"unrelated secret".to_vec()).unwrap();
    let small = engine.blob(b"small".to_vec()).unwrap();
    let first = put(&mut engine, first, b"small", file(small));
    let receipt = repository
        .commit_source(&engine, first, 0, "first")
        .unwrap();
    assert_eq!(receipt.before, initial);
    assert_eq!(receipt.after.root, first);
    assert_eq!(receipt.after.generation, 1);
    assert!(!object_ids(project.path()).contains(&orphan));
    let db = database(project.path());
    let inline: Vec<u8> = db
        .query_row(
            "SELECT payload FROM objects WHERE id = ?1",
            [small.to_bytes().unwrap().as_slice()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(inline, b"small");
    let payload: Vec<u8> = db
        .query_row(
            "SELECT payload FROM objects WHERE id = ?1",
            [large.to_bytes().unwrap().as_slice()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(payload, vec![3; FIXTURE_PAYLOAD_BYTES]);
    assert!(!project.path().join(".avc/objects").exists());
    drop(db);
    let second = put(&mut engine, first, b"file", Entry::Absent);
    let departure = repository
        .commit_source(&engine, second, 1, "second")
        .unwrap();
    drop(repository);
    let repository = Repository::open(project.path()).unwrap();
    let (selected, loaded) = repository.load().unwrap();
    assert_eq!(selected, departure.after);
    assert_eq!(loaded.read(second, &path(b"file")).unwrap(), Entry::Absent);
    assert!(loaded.object(orphan).is_err());
    assert!(loaded.object(large).is_err());
    let old = repository.load_root(departure.before.root).unwrap();
    assert_eq!(
        old.object(large).unwrap(),
        &Object::Blob(vec![3; FIXTURE_PAYLOAD_BYTES])
    );
    assert_eq!(old.read(first, &path(b"file")).unwrap(), file(large));
    assert_eq!(repository.receipt("first").unwrap(), Some(receipt));
    assert_eq!(repository.receipt("second").unwrap(), Some(departure));
}

#[test]
fn lost_response_retry_precedes_stale_and_engine_checks_and_key_identity_is_permanent() {
    let project = tempfile::tempdir().unwrap();
    let mut repository = Repository::init(project.path()).unwrap();
    let (mut engine, first, _) = source(b"first".to_vec());
    let original = repository
        .commit_source(&engine, first, 0, "stable")
        .unwrap();
    let second_blob = engine.blob(b"second".to_vec()).unwrap();
    let second = put(&mut engine, first, b"file", file(second_blob));
    repository.commit_source(&engine, second, 1, "new").unwrap();
    drop(repository);
    let mut repository = Repository::open(project.path()).unwrap();
    assert_eq!(
        repository
            .commit_source(&Engine::new(), first, 0, "stable")
            .unwrap(),
        original
    );
    assert_eq!(repository.status().unwrap().generation, 2);
    assert!(matches!(
        repository.commit_source(&engine, second, 0, "stable"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert!(matches!(
        repository.commit_source(&engine, first, 1, "stable"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert!(matches!(
        repository.commit_source(&engine, second, 0, "other"),
        Err(Error::Stale {
            expected: 0,
            actual: 2
        })
    ));
    assert_eq!(repository.status().unwrap().generation, 2);
    assert!(repository.receipt("other").unwrap().is_none());
}

#[test]
fn independent_connections_race_cas_and_same_content_admission() {
    let project = tempfile::tempdir().unwrap();
    drop(Repository::init(project.path()).unwrap());
    // Both roots share a payload; the loser must admit no candidate-only rows.
    let mut a = Repository::open(project.path()).unwrap();
    let mut b = Repository::open(project.path()).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = [(&mut a, "a"), (&mut b, "b")]
        .into_iter()
        .map(|(repo, key)| {
            let barrier = barrier.clone();
            let (mut engine, root, blob) = source(vec![9; FIXTURE_PAYLOAD_BYTES]);
            let root = put(&mut engine, root, key.as_bytes(), Entry::Symlink(blob));
            (repo, engine, root, key, barrier)
        })
        .collect();
    let outcomes = thread::scope(|scope| {
        handles
            .into_iter()
            .map(|(repo, engine, root, key, barrier)| {
                scope.spawn(move || {
                    barrier.wait();
                    repo.commit_source(&engine, root, 0, key)
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(outcomes.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        outcomes
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
    let winner = outcomes.into_iter().find_map(Result::ok).unwrap();
    assert_eq!(a.status().unwrap(), winner.after);
    assert_eq!(b.status().unwrap(), winner.after);
    let retained = object_ids(project.path());
    let selected = a.load_root(winner.after.root).unwrap();
    let initial = a.load_root(winner.before.root).unwrap();
    let expected: BTreeSet<_> = selected
        .objects()
        .map(|(id, _)| id)
        .chain(initial.objects().map(|(id, _)| id))
        .collect();
    assert_eq!(retained, expected);
}

#[test]
fn concurrent_identical_requests_return_one_receipt() {
    let project = tempfile::tempdir().unwrap();
    drop(Repository::init(project.path()).unwrap());
    let barrier = Arc::new(Barrier::new(2));
    let handles = (0..2)
        .map(|_| {
            let mut repository = Repository::open(project.path()).unwrap();
            let barrier = barrier.clone();
            thread::spawn(move || {
                let (engine, root, _) = source(vec![4; FIXTURE_PAYLOAD_BYTES]);
                barrier.wait();
                repository
                    .commit_source(&engine, root, 0, "identical")
                    .unwrap()
            })
        })
        .collect::<Vec<_>>();
    let receipts = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(receipts[0], receipts[1]);
    assert_eq!(receipts[0].after.generation, 1);
}

#[test]
fn damaged_objects_are_refused_without_silent_readmission_repair() {
    for damage in ["hash", "negative", "oversized", "mismatch", "missing"] {
        let project = tempfile::tempdir().unwrap();
        let mut repository = Repository::init(project.path()).unwrap();
        let (engine, root, blob) = source(vec![8; FIXTURE_PAYLOAD_BYTES]);
        repository
            .commit_source(&engine, root, 0, "original")
            .unwrap();
        let db = database(project.path());
        // Simulate damaged storage, not a supported mutation interface.
        db.execute_batch("DROP TRIGGER objects_no_update; DROP TRIGGER objects_no_delete; PRAGMA ignore_check_constraints = ON").unwrap();
        let sql = match damage {
            "hash" => "UPDATE objects SET payload = zeroblob(length) WHERE id = ?1",
            "negative" => "UPDATE objects SET length = -1 WHERE id = ?1",
            "oversized" => "UPDATE objects SET length = 16777217 WHERE id = ?1",
            "mismatch" => "UPDATE objects SET length = length - 1 WHERE id = ?1",
            "missing" => "DELETE FROM objects WHERE id = ?1",
            _ => unreachable!(),
        };
        db.execute(sql, [blob.to_bytes().unwrap().as_slice()])
            .unwrap();
        drop(db);
        let before = object_ids(project.path());
        assert!(repository.load().is_err(), "{damage}");
        assert!(repository.export(root, Vec::new()).is_err(), "{damage}");
        assert!(
            repository
                .commit_source(&engine, root, 1, "no-repair")
                .is_err(),
            "{damage}"
        );
        assert_eq!(repository.status().unwrap().generation, 1);
        assert!(repository.receipt("no-repair").unwrap().is_none());
        assert_eq!(object_ids(project.path()), before);
    }
}

#[test]
fn object_limit_refuses_before_selection_or_payload_admission() {
    let project = tempfile::tempdir().unwrap();
    let mut repository = Repository::init(project.path()).unwrap();
    let initial = repository.status().unwrap();
    let (engine, root, _) = source(vec![0; MAX_OBJECT_BYTES + 1]);
    assert!(matches!(
        repository.commit_source(&engine, root, 0, "too-large"),
        Err(Error::Limit("object bytes"))
    ));
    assert_eq!(repository.status().unwrap(), initial);
    assert_eq!(object_ids(project.path()).len(), 1);
    assert!(repository.receipt("too-large").unwrap().is_none());
}

#[test]
fn maximum_payload_fits_but_sqlite_rejects_oversized_rows_before_materialization() {
    let project = tempfile::tempdir().unwrap();
    let mut repository = Repository::init(project.path()).unwrap();
    let (engine, root, blob) = source(vec![42; MAX_OBJECT_BYTES]);
    let receipt = repository
        .commit_source(&engine, root, 0, "maximum")
        .unwrap();
    let loaded = repository.load_root(root).unwrap();
    assert_eq!(loaded.object(blob).unwrap(), engine.object(blob).unwrap());
    drop(loaded);
    let db = database(project.path());
    db.execute_batch("DROP TRIGGER objects_no_update; PRAGMA ignore_check_constraints = ON")
        .unwrap();
    db.execute(
        "UPDATE objects SET length = ?1, payload = zeroblob(?1) WHERE id = ?2",
        rusqlite::params![
            (MAX_OBJECT_BYTES + 2048) as i64,
            blob.to_bytes().unwrap().as_slice()
        ],
    )
    .unwrap();
    drop(db);
    assert!(
        matches!(repository.load_root(root), Err(Error::Sqlite(rusqlite::Error::SqliteFailure(error, _))) if error.code == rusqlite::ErrorCode::TooBig)
    );
    assert_eq!(repository.status().unwrap(), receipt.after);
    assert!(
        repository
            .commit_source(&engine, root, 1, "no-repair")
            .is_err()
    );
    assert!(repository.receipt("no-repair").unwrap().is_none());
}

// Independent fixture encoder for the documented object-only binary layout.
fn stream(root: Id, mut records: Vec<(Id, Vec<u8>)>) -> Vec<u8> {
    records.sort_by_key(|r| r.0);
    let mut bytes = b"AVCEXP01".to_vec();
    bytes.extend_from_slice(&root.to_bytes().unwrap());
    bytes.extend_from_slice(&(records.len() as u64).to_le_bytes());
    for (id, payload) in records {
        bytes.extend_from_slice(&id.to_bytes().unwrap());
        bytes.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&payload);
    }
    bytes.extend_from_slice(b"AVCEND01");
    let hash = blake3::hash(&bytes);
    bytes.extend_from_slice(hash.as_bytes());
    bytes
}

#[test]
fn independent_export_restore_includes_exact_source_closure_not_orphan_revisions() {
    let project = tempfile::tempdir().unwrap();
    let mut repository = Repository::init(project.path()).unwrap();
    let (mut engine, mut root, blob) = source(vec![2; FIXTURE_PAYLOAD_BYTES]);
    // More than 32 bindings exercises physical closure through radix branches.
    for i in 0..40 {
        root = put(
            &mut engine,
            root,
            format!("name-{i}").as_bytes(),
            file(blob),
        );
    }
    let parent_source = root;
    let small = engine.blob(b"symlink target".to_vec()).unwrap();
    root = put(&mut engine, root, b"symlink", Entry::Symlink(small));
    root = put(
        &mut engine,
        root,
        b"directory",
        Entry::Directory(parent_source),
    );
    let negative = engine.blob(b"base".to_vec()).unwrap();
    let conflict = Object::Conflict(Conflict {
        region: 0,
        positive: vec![(Entry::Absent, 1), (file(blob), 1)],
        negative: vec![(file(negative), 1)],
    });
    let conflict_id = conflict.id().unwrap();
    // Revision import admits preconstructed canonical objects, not authored by
    // repository initialization or commit_source.
    let parent = Object::Revision(Revision {
        root: parent_source,
        parents: vec![],
        change: [0; 16],
        author: "author".into(),
        message: "parent".into(),
        time_ns: 0,
        timezone_minutes: 0,
    });
    let parent_id = parent.id().unwrap();
    let mut records = engine
        .objects()
        .map(|(id, o)| (id, o.encode().unwrap()))
        .collect::<Vec<_>>();
    records.push((conflict_id, conflict.encode().unwrap()));
    let carrier = Object::Map(Node::Leaf(vec![(vec![6, 3], Entry::Conflict(conflict_id))]));
    let carrier_id = carrier.id().unwrap();
    records.push((carrier_id, carrier.encode().unwrap()));
    engine
        .import(records, carrier_id, Limits::default())
        .unwrap();
    root = put(&mut engine, root, b"conflict", Entry::Conflict(conflict_id));
    let revision = Object::Revision(Revision {
        root,
        parents: vec![parent_id],
        change: [1; 16],
        author: "author".into(),
        message: "child".into(),
        time_ns: 1,
        timezone_minutes: 0,
    });
    let revision_id = revision.id().unwrap();
    let mut records = engine
        .objects()
        .map(|(id, o)| (id, o.encode().unwrap()))
        .collect::<Vec<_>>();
    records.push((parent_id, parent.encode().unwrap()));
    records.push((revision_id, revision.encode().unwrap()));
    engine
        .import(records, revision_id, Limits::default())
        .unwrap();
    let orphan = engine.blob(b"not part of this export".to_vec()).unwrap();
    assert!(matches!(
        repository.commit_source(&engine, revision_id, 0, "revision"),
        Err(Error::Invalid(_))
    ));
    repository
        .commit_source(&engine, root, 0, "source")
        .unwrap();
    let loaded = repository.load_root(root).unwrap();
    assert!(loaded.object(orphan).is_err());
    assert!(loaded.object(parent_id).is_err());
    assert!(loaded.object(revision_id).is_err());
    let mut exported = Vec::new();
    repository.export(root, &mut exported).unwrap();
    let expected_records = loaded
        .objects()
        .map(|(id, object)| (id, object.encode().unwrap()))
        .collect();
    assert_eq!(exported, stream(root, expected_records));
    drop(repository);
    // Restore is independent even after all source store files are removed.
    fs::remove_dir_all(project.path().join(".avc")).unwrap();
    let destination = tempfile::tempdir().unwrap();
    let mut restored = Repository::init(destination.path()).unwrap();
    let receipt = restored.import(Cursor::new(&exported), "restore").unwrap();
    assert_eq!(receipt.after.root, root);
    drop(restored);
    let mut restored = Repository::open(destination.path()).unwrap();
    let (selection, restored_engine) = restored.load().unwrap();
    assert_eq!(selection.root, root);
    assert_eq!(
        restored_engine.read(root, &path(b"file")).unwrap(),
        file(blob)
    );
    assert_eq!(
        restored_engine.object(negative).unwrap(),
        &Object::Blob(b"base".to_vec())
    );
    assert_eq!(
        restored_engine.object(blob).unwrap(),
        &Object::Blob(vec![2; FIXTURE_PAYLOAD_BYTES])
    );
    assert!(restored_engine.object(parent_id).is_err());
    assert!(restored_engine.diff(parent_source, root).unwrap().len() >= 3);
    assert_eq!(
        restored.import(Cursor::new(&exported), "restore").unwrap(),
        receipt
    );
    assert!(matches!(
        restored.import(Cursor::new(&exported), "different"),
        Err(Error::NotEmpty)
    ));
}

#[test]
fn source_root_conflict_roundtrip_owns_both_sides_and_negative_base() {
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let left_blob = engine.blob(b"left".to_vec()).unwrap();
    let right_blob = engine.blob(b"right".to_vec()).unwrap();
    let left = put(&mut engine, empty, b"file", file(left_blob));
    let right = put(&mut engine, empty, b"file", file(right_blob));
    let mut positive = vec![(Entry::Directory(left), 1), (Entry::Directory(right), 1)];
    positive.sort_by_key(|(entry, _)| entry.encode().unwrap());
    let conflict = Object::Conflict(Conflict {
        region: 1,
        positive,
        negative: vec![(Entry::Directory(empty), 1)],
    });
    let root = conflict.id().unwrap();
    let mut records = engine
        .objects()
        .map(|(id, o)| (id, o.encode().unwrap()))
        .collect::<Vec<_>>();
    records.push((root, conflict.encode().unwrap()));
    engine.import(records, root, Limits::default()).unwrap();
    let project = tempfile::tempdir().unwrap();
    let mut repository = Repository::init(project.path()).unwrap();
    repository
        .commit_source(&engine, root, 0, "conflicted")
        .unwrap();
    let loaded = repository.load().unwrap().1;
    assert_eq!(loaded.object(root).unwrap(), &conflict);
    assert_eq!(
        loaded.object(left_blob).unwrap(),
        &Object::Blob(b"left".to_vec())
    );
    assert_eq!(
        loaded.object(right_blob).unwrap(),
        &Object::Blob(b"right".to_vec())
    );
    assert_eq!(
        loaded.object(empty).unwrap(),
        &Object::Map(Node::Leaf(vec![]))
    );
    let mut exported = Vec::new();
    repository.export(root, &mut exported).unwrap();
    let destination = tempfile::tempdir().unwrap();
    let mut restored = Repository::init(destination.path()).unwrap();
    restored
        .import(Cursor::new(exported), "conflicted")
        .unwrap();
    assert_eq!(restored.load().unwrap().1.object(root).unwrap(), &conflict);
}

#[test]
fn import_refuses_incomplete_changed_surplus_missing_and_oversized_streams_before_admission() {
    let (engine, root, blob) = source(b"fixture".to_vec());
    let records = engine
        .objects()
        .filter(|(id, _)| *id != Id::hash(avc::MAP_LEAF_KIND, &[0]).unwrap())
        .map(|(id, o)| (id, o.encode().unwrap()))
        .collect::<Vec<_>>();
    let valid = stream(root, records.clone());
    let mut variants = vec![valid[..valid.len() - 1].to_vec(), valid[..49].to_vec()];
    let mut trailing = valid.clone();
    trailing.push(0);
    variants.push(trailing);
    let mut hash = valid.clone();
    *hash.last_mut().unwrap() ^= 1;
    variants.push(hash);
    let mut payload = valid.clone();
    payload[90] ^= 1;
    variants.push(payload);
    let orphan = Object::Blob(b"orphan".to_vec());
    let mut surplus = records.clone();
    surplus.push((orphan.id().unwrap(), orphan.encode().unwrap()));
    variants.push(stream(root, surplus));
    variants.push(stream(
        root,
        records
            .iter()
            .filter(|(id, _)| *id != blob)
            .cloned()
            .collect(),
    ));
    // An attacker can repair the outer stream hash: typed hashes must still fail.
    let mut bad_typed = records.clone();
    bad_typed.iter_mut().find(|(id, _)| *id == blob).unwrap().1[0] ^= 1;
    variants.push(stream(root, bad_typed));
    let mut oversized = valid.clone();
    oversized[82..90].copy_from_slice(&((MAX_OBJECT_BYTES as u64) + 1).to_le_bytes());
    variants.push(oversized);
    let mut excessive_count = valid.clone();
    excessive_count[41..49].copy_from_slice(&u64::MAX.to_le_bytes());
    variants.push(excessive_count);
    let mut version = valid.clone();
    version[7] = b'2';
    variants.push(version);
    variants.push(stream(blob, vec![(blob, b"fixture".to_vec())]));
    let invalid_map = Object::Map(Node::Leaf(vec![(vec![6], file(blob))]));
    let invalid_root = invalid_map.id().unwrap();
    variants.push(stream(
        invalid_root,
        vec![
            (blob, b"fixture".to_vec()),
            (invalid_root, invalid_map.encode().unwrap()),
        ],
    ));
    for bytes in variants {
        let project = tempfile::tempdir().unwrap();
        let mut repository = Repository::init(project.path()).unwrap();
        let initial = repository.status().unwrap();
        assert!(repository.import(Cursor::new(bytes), "untrusted").is_err());
        assert_eq!(repository.status().unwrap(), initial);
        assert_eq!(object_ids(project.path()).len(), 1);
        assert!(repository.receipt("untrusted").unwrap().is_none());
    }
    let project = tempfile::tempdir().unwrap();
    let mut repository = Repository::init(project.path()).unwrap();
    repository.import(Cursor::new(valid), "valid").unwrap();
    assert_eq!(repository.load().unwrap().0.root, root);
}

#[cfg(unix)]
#[test]
fn reopen_cannot_bypass_failed_repository_directory_sync() {
    use std::os::unix::fs::PermissionsExt;
    let project = tempfile::tempdir().unwrap();
    struct Reset<'a>(&'a Path);
    impl Drop for Reset<'_> {
        fn drop(&mut self) {
            fs::set_permissions(self.0, fs::Permissions::from_mode(0o700)).unwrap();
        }
    }
    fs::set_permissions(project.path(), fs::Permissions::from_mode(0o300)).unwrap();
    let reset = Reset(project.path());
    // A privileged principal/filesystem may still read this directory, in
    // which case this mechanism cannot inject the requested sync-open failure.
    if std::fs::File::open(project.path()).is_ok() {
        return;
    }
    assert!(Repository::init(project.path()).is_err());
    assert!(
        Repository::open(project.path()).is_err(),
        "open must finish installation sync before permitting acknowledgements"
    );
    drop(reset);
    let mut reopened = Repository::open(project.path()).unwrap();
    let (engine, root, _) = source(b"after completed installation".to_vec());
    reopened
        .commit_source(&engine, root, 0, "recovered-init")
        .unwrap();
}

#[test]
fn export_write_failure_is_reported_and_does_not_change_the_store() {
    struct Fail;
    impl Write for Fail {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(
                io::ErrorKind::StorageFull,
                "injected full export sink",
            ))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let project = tempfile::tempdir().unwrap();
    let repository = Repository::init(project.path()).unwrap();
    let initial = repository.status().unwrap();
    assert!(
        matches!(repository.export(initial.root, Fail), Err(Error::Io(e)) if e.kind() == io::ErrorKind::StorageFull)
    );
    assert_eq!(repository.status().unwrap(), initial);
}
