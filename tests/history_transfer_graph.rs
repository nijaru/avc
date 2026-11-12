use avc::{
    Conflict, Entry, Id, Object, Revision,
    engine::{Edit, Engine},
    repository::{Error, Repository},
};
use std::collections::BTreeMap;

fn literal(root: Id, head: Id, records: &BTreeMap<Id, Vec<u8>>) -> Vec<u8> {
    let mut out = b"AVCHIS01".to_vec();
    out.extend(root.to_bytes().unwrap());
    out.push(1);
    out.extend(head.to_bytes().unwrap());
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

#[test]
fn history_transfer_owns_all_ordered_merge_parents_and_root_conflict_terms() {
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let mut root = |bytes: &[u8]| {
        let blob = engine.blob(bytes.to_vec()).unwrap();
        engine
            .edit(
                empty,
                &[Edit {
                    path: vec![b"f".to_vec()],
                    expected: Entry::Absent,
                    value: Entry::File {
                        blob,
                        executable: false,
                    },
                }],
            )
            .unwrap()
    };
    let left = root(b"left");
    let right = root(b"right");
    let mut records: BTreeMap<_, _> = engine
        .objects()
        .map(|(id, o)| (id, o.encode().unwrap()))
        .collect();
    let mut positive = vec![(Entry::Directory(left), 1), (Entry::Directory(right), 1)];
    positive.sort_by_key(|(e, _)| e.encode().unwrap());
    let conflict = Object::Conflict(Conflict {
        region: 1,
        positive,
        negative: vec![(Entry::Directory(empty), 1)],
    });
    let conflict_id = conflict.id().unwrap();
    records.insert(conflict_id, conflict.encode().unwrap());
    let mut revision = |root, parents, change| {
        let object = Object::Revision(Revision {
            root,
            parents,
            change: [change; 16],
            author: "Independent fixture".into(),
            message: "preserve exact graph".into(),
            time_ns: 0,
            timezone_minutes: 0,
        });
        let id = object.id().unwrap();
        records.insert(id, object.encode().unwrap());
        id
    };
    let first = revision(left, vec![], 1);
    let second = revision(right, vec![], 2);
    let head = revision(conflict_id, vec![first, second], 3);
    let input = literal(left, head, &records);
    let fresh = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(fresh.path()).unwrap();
    let receipt = repo.import_history(input.as_slice(), "restore").unwrap();
    assert_eq!(receipt.after.revision, Some(head));
    assert_eq!(receipt.after.root, left);
    assert_eq!(repo.revision(head).unwrap().parents, vec![first, second]);
    assert_eq!(
        repo.log(None, 20)
            .unwrap()
            .iter()
            .map(|r| r.0)
            .collect::<Vec<_>>(),
        vec![head, first]
    );
    let loaded = repo.load_root(head).unwrap();
    assert_eq!(loaded.object(conflict_id).unwrap(), &conflict);
    assert!(loaded.object(second).is_ok());
    assert!(loaded.object(empty).is_ok());
    let mut output = Vec::new();
    repo.export_history(&mut output).unwrap();
    assert_eq!(output, input);
    let changed_source = literal(right, head, &records);
    assert!(matches!(
        repo.import_history(changed_source.as_slice(), "restore"),
        Err(Error::RequestKeyReuse(_))
    ));
    let parent_records = repo
        .load_root(first)
        .unwrap()
        .objects()
        .map(|(id, o)| (id, o.encode().unwrap()))
        .collect();
    assert!(matches!(
        repo.import_history(literal(left, first, &parent_records).as_slice(), "restore"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert_eq!(repo.status().unwrap(), receipt.after);
    // Losing a non-first parent is corruption, not a valid first-parent-only backup.
    records.remove(&second);
    let missing = literal(left, head, &records);
    let other = tempfile::tempdir().unwrap();
    let mut refused = Repository::init(other.path()).unwrap();
    assert!(refused.import_history(missing.as_slice(), "bad").is_err());
    assert_eq!(refused.status().unwrap().generation, 0);
    assert!(refused.receipt("bad").unwrap().is_none());
}
