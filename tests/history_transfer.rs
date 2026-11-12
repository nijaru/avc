use avc::{
    Entry, Id, Object,
    engine::{Edit, Engine},
    repository::{Error, MAX_OBJECT_BYTES, MAX_OBJECTS, Operation, Repository},
};
use std::{
    collections::BTreeMap,
    fs,
    sync::{Arc, Barrier},
};

type Records = BTreeMap<Id, Vec<u8>>;
fn source(bytes: &[u8]) -> (Engine, Id) {
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
                    executable: true,
                },
            }],
        )
        .unwrap();
    (engine, root)
}
fn select(repo: &mut Repository, bytes: &[u8], key: &str) -> Id {
    let (engine, root) = source(bytes);
    repo.commit_source(&engine, root, repo.status().unwrap().generation, key)
        .unwrap();
    root
}
fn fixture() -> (tempfile::TempDir, Repository, Vec<u8>) {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    select(&mut repo, b"orphan", "discarded");
    select(&mut repo, b"first", "select-one");
    repo.commit("Author", "first\nmessage", 2, "author-one")
        .unwrap();
    select(&mut repo, b"second", "select-two");
    repo.commit("Other Author", "second", 4, "author-two")
        .unwrap();
    select(
        &mut repo,
        b"working source, not yet authored\0\r\n",
        "uncommitted",
    );
    let mut wire = Vec::new();
    repo.export_history(&mut wire).unwrap();
    (project, repo, wire)
}
// Literal independent framing, not the production transfer writer.
fn wire(root: Id, head: Option<Id>, records: &[(Id, Vec<u8>)]) -> Vec<u8> {
    let mut out = b"AVCHIS01".to_vec();
    out.extend(root.to_bytes().unwrap());
    out.push(u8::from(head.is_some()));
    if let Some(id) = head {
        out.extend(id.to_bytes().unwrap());
    }
    out.extend((records.len() as u64).to_le_bytes());
    for (id, payload) in records {
        out.extend(id.to_bytes().unwrap());
        out.extend((payload.len() as u64).to_le_bytes());
        out.extend(payload);
    }
    out.extend(b"AVCHEND1");
    out.extend(blake3::hash(&out).as_bytes());
    out
}
fn parse(bytes: &[u8]) -> (Id, Option<Id>, Records) {
    assert_eq!(&bytes[..8], b"AVCHIS01");
    let root = Id::from_bytes(&bytes[8..41]).unwrap();
    let mut offset = 42;
    let head = match bytes[41] {
        0 => None,
        1 => {
            offset += 33;
            Some(Id::from_bytes(&bytes[42..75]).unwrap())
        }
        _ => panic!(),
    };
    let count = u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
    offset += 8;
    let mut records = BTreeMap::new();
    for _ in 0..count {
        let id = Id::from_bytes(&bytes[offset..offset + 33]).unwrap();
        offset += 33;
        let length = u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap()) as usize;
        offset += 8;
        assert!(
            records
                .insert(id, bytes[offset..offset + length].to_vec())
                .is_none()
        );
        offset += length;
    }
    assert_eq!(&bytes[offset..offset + 8], b"AVCHEND1");
    assert_eq!(
        &bytes[offset + 8..],
        blake3::hash(&bytes[..offset + 8]).as_bytes()
    );
    (root, head, records)
}
fn bytes(repo: &Repository, root: Id) -> Vec<u8> {
    let engine = repo.load_root(root).unwrap();
    let Entry::File { blob, executable } = engine.read(root, &[b"file".to_vec()]).unwrap() else {
        panic!()
    };
    assert!(executable);
    let Object::Blob(bytes) = engine.object(blob).unwrap() else {
        panic!()
    };
    bytes.clone()
}

#[test]
fn selected_history_roundtrip_preserves_ancestry_dirty_selection_and_retry_identity() {
    let (original, repo, wire) = fixture();
    let selected = repo.status().unwrap();
    let log = repo.log(None, 20).unwrap();
    let (root, head, records) = parse(&wire);
    assert_eq!((root, head), (selected.root, selected.revision));
    assert_ne!(root, log[0].1.root);
    let (_, orphan) = source(b"orphan");
    assert!(!records.contains_key(&orphan));
    assert_eq!(
        records
            .values()
            .filter(|p| p.as_slice() == b"orphan")
            .count(),
        0
    );
    let fresh = tempfile::tempdir().unwrap();
    let mut restored = Repository::init(fresh.path()).unwrap();
    let receipt = restored.import_history(wire.as_slice(), "restore").unwrap();
    assert_eq!(receipt.operation, Operation::ImportHistory);
    assert_eq!(receipt.before.generation, 0);
    assert_eq!(receipt.after.generation, 1);
    assert_eq!((receipt.after.root, receipt.after.revision), (root, head));
    assert_eq!(restored.log(None, 20).unwrap(), log);
    assert_eq!(
        bytes(&restored, root),
        b"working source, not yet authored\0\r\n"
    );
    assert_eq!(bytes(&restored, log[0].1.root), b"second");
    assert_eq!(bytes(&restored, log[1].1.root), b"first");
    for key in [
        "author-one",
        "author-two",
        "discarded",
        "select-one",
        "uncommitted",
    ] {
        assert!(restored.receipt(key).unwrap().is_none());
    }
    let instance = |path: &std::path::Path| {
        rusqlite::Connection::open(path.join(".avc/store.sqlite"))
            .unwrap()
            .query_row::<Vec<u8>, _, _>("SELECT instance FROM repository", [], |r| r.get(0))
            .unwrap()
    };
    assert_ne!(instance(original.path()), instance(fresh.path()));
    let mut reexport = Vec::new();
    restored.export_history(&mut reexport).unwrap();
    assert_eq!(reexport, wire); // No instance, generation or regenerated authored metadata.
    select(&mut restored, b"later", "later-source");
    restored
        .commit("Author", "later", 2, "later-author")
        .unwrap();
    let current = restored.status().unwrap();
    assert_eq!(
        restored.import_history(wire.as_slice(), "restore").unwrap(),
        receipt
    );
    assert_eq!(restored.status().unwrap(), current);
    assert!(matches!(
        restored.import_history(wire.as_slice(), "different"),
        Err(Error::NotEmpty)
    ));
    assert!(matches!(
        restored.commit("Author", "x", 0, "restore"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert!(matches!(
        restored.commit_source(&Engine::new(), root, 0, "restore"),
        Err(Error::RequestKeyReuse(_))
    ));
    drop(restored);
    let restored = Repository::open(fresh.path()).unwrap();
    assert_eq!(restored.status().unwrap(), current);
    assert_eq!(restored.receipt("restore").unwrap(), Some(receipt));
    restored.load_root(current.revision.unwrap()).unwrap();
}

#[test]
fn unborn_history_and_explicit_stream_modes_remain_distinct() {
    for changed in [false, true] {
        let project = tempfile::tempdir().unwrap();
        let mut repo = Repository::init(project.path()).unwrap();
        if changed {
            select(&mut repo, b"unborn source", "source");
        }
        let mut history = Vec::new();
        let selected = repo.export_history(&mut history).unwrap();
        assert_eq!(parse(&history).1, None);
        let mut source = Vec::new();
        repo.export(selected.root, &mut source).unwrap();
        let fresh = tempfile::tempdir().unwrap();
        let mut restored = Repository::init(fresh.path()).unwrap();
        assert!(restored.import(history.as_slice(), "wrong-mode").is_err());
        assert!(
            restored
                .import_history(source.as_slice(), "wrong-mode")
                .is_err()
        );
        assert_eq!(restored.status().unwrap().generation, 0);
        restored
            .import_history(history.as_slice(), "restore")
            .unwrap();
        assert_eq!(restored.status().unwrap().root, selected.root);
        assert_eq!(restored.status().unwrap().revision, None);
        assert!(restored.log(None, 20).unwrap().is_empty());
        let mut exact_source = Vec::new();
        restored.export(selected.root, &mut exact_source).unwrap();
        assert_eq!(source, exact_source);
    }
}

#[test]
fn invalid_history_never_admits_objects_or_advances_selection() {
    let (_project, _repo, valid) = fixture();
    let (root, head, records) = parse(&valid);
    let records: Vec<_> = records.into_iter().collect();
    let mut invalid = vec![valid[..valid.len() - 1].to_vec(), valid[..42].to_vec()];
    let mut trailing = valid.clone();
    trailing.push(0);
    invalid.push(trailing);
    let mut tag = valid.clone();
    tag[41] = 2;
    invalid.push(tag);
    invalid.push(wire(root, Some(root), &records));
    invalid.push(wire(records[0].0, head, &records)); // Blob is not a selected source.
    let mut reversed = records.clone();
    reversed.reverse();
    invalid.push(wire(root, head, &reversed));
    let mut duplicate = records.clone();
    duplicate.insert(0, records[0].clone());
    invalid.push(wire(root, head, &duplicate));
    for missing in [root, head.unwrap()] {
        let kept: Vec<_> = records
            .iter()
            .filter(|(id, _)| *id != missing)
            .cloned()
            .collect();
        invalid.push(wire(root, head, &kept));
    }
    let parent = match Object::decode(
        head.unwrap(),
        &records.iter().find(|(id, _)| Some(*id) == head).unwrap().1,
    )
    .unwrap()
    {
        Object::Revision(rev) => rev.parents[0],
        _ => panic!(),
    };
    let kept: Vec<_> = records
        .iter()
        .filter(|(id, _)| *id != parent)
        .cloned()
        .collect();
    invalid.push(wire(root, head, &kept));
    let extra = Object::Blob(b"not-owned".to_vec());
    let mut surplus = records.clone();
    surplus.push((extra.id().unwrap(), extra.encode().unwrap()));
    surplus.sort_by_key(|r| r.0);
    invalid.push(wire(root, head, &surplus));
    let mut corrupt = records.clone();
    corrupt[0].1[0] ^= 1;
    invalid.push(wire(root, head, &corrupt));
    let mut count = valid.clone();
    count[75..83].copy_from_slice(&(MAX_OBJECTS as u64 + 1).to_le_bytes());
    invalid.push(count);
    let mut length = valid.clone();
    length[116..124].copy_from_slice(&(MAX_OBJECT_BYTES as u64 + 1).to_le_bytes());
    invalid.push(length);
    let fresh = tempfile::tempdir().unwrap();
    let mut restored = Repository::init(fresh.path()).unwrap();
    let before = restored.status().unwrap();
    for (i, input) in invalid.iter().enumerate() {
        assert!(
            restored.import_history(input.as_slice(), "bad").is_err(),
            "invalid case {i} accepted"
        );
        assert_eq!(restored.status().unwrap(), before);
        assert!(restored.receipt("bad").unwrap().is_none());
    }
    let db = rusqlite::Connection::open(fresh.path().join(".avc/store.sqlite")).unwrap();
    assert_eq!(
        db.query_row::<i64, _, _>("SELECT count(*) FROM objects", [], |r| r.get(0))
            .unwrap(),
        1
    );
    restored.import_history(valid.as_slice(), "bad").unwrap();
}

#[test]
fn concurrent_history_restore_is_exactly_once_and_cross_kind_reuse_refuses() {
    let (_project, _repo, input) = fixture();
    for identical in [false, true] {
        let fresh = tempfile::tempdir().unwrap();
        drop(Repository::init(fresh.path()).unwrap());
        let barrier = Arc::new(Barrier::new(2));
        let workers = (0..2)
            .map(|i| {
                let mut repo = Repository::open(fresh.path()).unwrap();
                let input = input.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    repo.import_history(
                        input.as_slice(),
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
            assert!(results.iter().any(|r| matches!(r, Err(Error::NotEmpty))));
        }
        let repo = Repository::open(fresh.path()).unwrap();
        assert_eq!(repo.status().unwrap().generation, 1);
        assert_eq!(repo.log(None, 20).unwrap().len(), 2);
    }
    let fresh = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(fresh.path()).unwrap();
    repo.commit("Author", "already used", 0, "used").unwrap();
    assert!(matches!(
        repo.import_history(input.as_slice(), "used"),
        Err(Error::RequestKeyReuse(_))
    ));
}

#[test]
fn rejected_history_receipt_rolls_back_objects_and_both_selections() {
    let (_original, _repo, input) = fixture();
    let fresh = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(fresh.path()).unwrap();
    let before = repo.status().unwrap();
    let db = rusqlite::Connection::open(fresh.path().join(".avc/store.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER deny_history BEFORE INSERT ON operations WHEN NEW.kind = 'import-history' BEGIN SELECT RAISE(ABORT, 'denied history'); END;").unwrap();
    assert!(repo.import_history(input.as_slice(), "restore").is_err());
    assert_eq!(repo.status().unwrap(), before);
    assert!(repo.receipt("restore").unwrap().is_none());
    assert_eq!(
        db.query_row::<i64, _, _>("SELECT count(*) FROM objects", [], |r| r.get(0))
            .unwrap(),
        1
    );
    db.execute_batch("DROP TRIGGER deny_history").unwrap();
    repo.import_history(input.as_slice(), "restore").unwrap();
}

#[test]
fn history_export_preflights_all_ancestry_and_never_clobbers_or_repairs_damage() {
    let (project, repo, _input) = fixture();
    let log = repo.log(None, 20).unwrap();
    let engine = repo.load_root(log[1].1.root).unwrap();
    let Entry::File { blob, .. } = engine.read(log[1].1.root, &[b"file".to_vec()]).unwrap() else {
        panic!()
    };
    let path = project.path().join("history.avc");
    repo.export_history_file(&path).unwrap();
    let original = fs::read(&path).unwrap();
    assert!(repo.export_history_file(&path).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    let db = rusqlite::Connection::open(project.path().join(".avc/store.sqlite")).unwrap();
    db.execute_batch("DROP TRIGGER objects_no_update").unwrap();
    db.execute(
        "UPDATE objects SET payload = ?1 WHERE id = ?2",
        rusqlite::params![b"wrong".as_slice(), blob.to_bytes().unwrap().as_slice()],
    )
    .unwrap();
    let mut rejected = Vec::new();
    assert!(repo.export_history(&mut rejected).is_err());
    assert!(rejected.is_empty());
    let bad = project.path().join("must-not-create.avc");
    assert!(repo.export_history_file(&bad).is_err());
    assert!(!bad.exists());
}
