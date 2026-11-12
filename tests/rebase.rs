use avc::{
    Conflict, Entry, Id, Object, Revision,
    engine::{Edit, Engine},
    repository::{Error, Operation, Receipt, Repository},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{Arc, Barrier},
};

fn select(repo: &mut Repository, files: &[(&str, &[u8])], key: &str) -> Receipt {
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let edits = files
        .iter()
        .map(|(name, bytes)| {
            let blob = engine.blob(bytes.to_vec()).unwrap();
            Edit {
                path: vec![name.as_bytes().to_vec()],
                expected: Entry::Absent,
                value: Entry::File {
                    blob,
                    executable: false,
                },
            }
        })
        .collect::<Vec<_>>();
    let root = engine.edit(empty, &edits).unwrap();
    repo.commit_source(&engine, root, repo.status().unwrap().generation, key)
        .unwrap()
}
fn commit(repo: &mut Repository, key: &str) -> Id {
    repo.commit(
        "Ada <ada@example.test>",
        key,
        repo.status().unwrap().generation,
        key,
    )
    .unwrap()
    .after
    .revision
    .unwrap()
}
fn branches() -> (tempfile::TempDir, Repository, Id, Id, Id) {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(dir.path()).unwrap();
    select(&mut repo, &[("base", b"base\n")], "base-source");
    let base = commit(&mut repo, "base");
    select(
        &mut repo,
        &[("base", b"base\n"), ("upstream", b"upstream\n")],
        "onto-source",
    );
    let onto = commit(&mut repo, "onto");
    repo.undo("onto", 4, "back-to-base").unwrap();
    select(
        &mut repo,
        &[("base", b"base\n"), ("feature", b"feature\n")],
        "tip-source",
    );
    let tip = commit(&mut repo, "tip");
    (dir, repo, base, onto, tip)
}
fn ids(path: &Path) -> BTreeSet<Vec<u8>> {
    let db = rusqlite::Connection::open(path.join(".avc/store.sqlite")).unwrap();
    db.prepare("SELECT id FROM objects")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}
fn files(repo: &Repository, root: Id) -> BTreeMap<Vec<u8>, Vec<u8>> {
    let engine = repo.load_root(root).unwrap();
    engine
        .list(root, &[])
        .unwrap()
        .into_iter()
        .map(|(name, entry)| {
            let Entry::File { blob, .. } = entry else {
                panic!("expected file")
            };
            let Object::Blob(bytes) = engine.object(blob).unwrap() else {
                panic!("expected blob")
            };
            (name, bytes.clone())
        })
        .collect()
}

#[test]
fn replay_preserves_authorship_exact_new_parent_and_independent_source_then_undoes() {
    let (dir, mut repo, base, onto, tip) = branches();
    let before = repo.status().unwrap();
    let original = repo.revision(tip).unwrap();
    let live = dir.path().join("live");
    std::fs::write(&live, b"never selected").unwrap();
    let replay = repo.rebase(onto, before.generation, "rebase").unwrap();
    assert_eq!(replay.operation, Operation::Rebase { onto });
    assert_eq!(replay.before, before);
    assert_eq!(replay.after.generation, before.generation + 1);
    let id = replay.after.revision.unwrap();
    assert_ne!(id, tip);
    let mut expected = original.clone();
    expected.parents = vec![onto];
    expected.root = replay.after.root;
    assert_eq!(repo.revision(id).unwrap(), expected);
    assert_eq!(repo.revision(tip).unwrap(), original);
    assert_eq!(
        repo.log(None, 20)
            .unwrap()
            .iter()
            .map(|r| r.0)
            .collect::<Vec<_>>(),
        vec![id, onto, base]
    );
    assert_eq!(
        files(&repo, replay.after.root),
        BTreeMap::from([
            (b"base".to_vec(), b"base\n".to_vec()),
            (b"feature".to_vec(), b"feature\n".to_vec()),
            (b"upstream".to_vec(), b"upstream\n".to_vec()),
        ])
    );
    let undone = repo
        .undo("rebase", replay.after.generation, "undo-rebase")
        .unwrap();
    assert_eq!(
        (undone.after.root, undone.after.revision),
        (before.root, before.revision)
    );
    let redone = repo
        .undo("undo-rebase", undone.after.generation, "redo-rebase")
        .unwrap();
    assert_eq!(
        (redone.after.root, redone.after.revision),
        (replay.after.root, Some(id))
    );
    assert_eq!(std::fs::read(live).unwrap(), b"never selected");
    select(&mut repo, &[("later", b"dirty")], "later");
    let later = repo.status().unwrap();
    assert_eq!(
        repo.rebase(onto, before.generation, "rebase").unwrap(),
        replay
    );
    for (target, generation) in [
        (base, before.generation),
        (onto, before.generation + 1),
        (before.root, before.generation),
    ] {
        assert!(matches!(
            repo.rebase(target, generation, "rebase"),
            Err(Error::RequestKeyReuse(_))
        ));
    }
    assert!(matches!(
        repo.commit("Ada", "changed", before.generation, "rebase"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert!(matches!(
        repo.rebase(onto, before.generation, "tip"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert_eq!(repo.status().unwrap(), later);
    drop(repo);
    let mut repo = Repository::open(dir.path()).unwrap();
    assert_eq!(repo.receipt("rebase").unwrap(), Some(replay.clone()));
    assert_eq!(
        repo.rebase(onto, before.generation, "rebase").unwrap(),
        replay
    );
}

#[test]
fn exact_existing_parent_is_a_deterministic_noop_with_a_forward_receipt() {
    let (dir, mut repo, base, _, tip) = branches();
    let before = repo.status().unwrap();
    let retained = ids(dir.path());
    let receipt = repo.rebase(base, before.generation, "noop").unwrap();
    assert_eq!(receipt.after.revision, Some(tip));
    assert_eq!(receipt.after.root, before.root);
    assert_eq!(receipt.after.generation, before.generation + 1);
    assert_eq!(ids(dir.path()), retained);
    assert_eq!(
        repo.rebase(base, before.generation, "noop").unwrap(),
        receipt
    );
    assert_eq!(
        repo.undo("noop", receipt.after.generation, "undo-noop")
            .unwrap()
            .after
            .revision,
        Some(tip)
    );
}

#[test]
fn dirty_unborn_unknown_wrong_kind_and_same_change_ancestry_refuse_without_admission() {
    let (dir, mut repo, _, onto, tip) = branches();
    let original = repo.status().unwrap();
    let retained = ids(dir.path());
    for target in [
        tip,
        original.root,
        Id {
            kind: 5,
            digest: [255; 32],
        },
    ] {
        assert!(repo.rebase(target, original.generation, "refused").is_err());
        assert_eq!(repo.status().unwrap(), original);
        assert!(repo.receipt("refused").unwrap().is_none());
        assert_eq!(ids(dir.path()), retained);
    }
    let descendant = commit(&mut repo, "descendant");
    repo.undo("descendant", original.generation + 1, "unselect-descendant")
        .unwrap();
    let current = repo.status().unwrap();
    assert!(
        repo.rebase(descendant, current.generation, "descendant-target")
            .is_err()
    );
    select(&mut repo, &[("dirty", b"not authored")], "dirty");
    let dirty = repo.status().unwrap();
    let retained = ids(dir.path());
    assert!(
        repo.rebase(onto, dirty.generation, "dirty-refusal")
            .is_err()
    );
    assert_eq!(repo.status().unwrap(), dirty);
    assert_eq!(ids(dir.path()), retained);
    let unborn = tempfile::tempdir().unwrap();
    let mut empty = Repository::init(unborn.path()).unwrap();
    assert!(empty.rebase(onto, 0, "unborn").is_err());
    assert_eq!(empty.status().unwrap().generation, 0);
}

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
fn conflicted_base(resolve_right: bool, merge_head: bool) -> (Vec<u8>, Id, Id) {
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let mut source = |bytes: &[u8]| {
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
    let left = source(b"\0left");
    let right = source(b"\0right");
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
            author: "Imported".into(),
            message: "Original authored metadata".into(),
            time_ns: -12345,
            timezone_minutes: -210,
        });
        let id = object.id().unwrap();
        records.insert(id, object.encode().unwrap());
        id
    };
    let onto = revision(right, vec![], 1);
    let base = revision(conflict_id, vec![onto], 2);
    let root = if resolve_right { right } else { left };
    let head = revision(
        root,
        if merge_head {
            vec![base, onto]
        } else {
            vec![base]
        },
        3,
    );
    (literal(root, head, &records), onto, conflict_id)
}

#[test]
fn differing_resolutions_of_conflicted_base_refuse_but_equal_resolution_fast_path_is_safe() {
    for equal in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = Repository::init(dir.path()).unwrap();
        let (stream, onto, conflict) = conflicted_base(equal, false);
        repo.import_history(stream.as_slice(), "fixture").unwrap();
        let before = repo.status().unwrap();
        let retained = ids(dir.path());
        let result = repo.rebase(onto, before.generation, "rebase");
        if equal {
            let receipt = result.unwrap();
            let revised = repo.revision(receipt.after.revision.unwrap()).unwrap();
            assert_eq!(revised.root, repo.revision(onto).unwrap().root);
            assert_eq!(revised.change, [3; 16]);
            assert_eq!(revised.parents, vec![onto]);
            assert_eq!((revised.time_ns, revised.timezone_minutes), (-12345, -210));
        } else {
            assert!(
                matches!(result, Err(Error::Source(avc::Error::UnresolvedBase(id))) if id == conflict)
            );
            assert_eq!(repo.status().unwrap(), before);
            assert_eq!(ids(dir.path()), retained);
            assert!(repo.receipt("rebase").unwrap().is_none());
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(dir.path()).unwrap();
    let (stream, onto, _) = conflicted_base(false, true);
    repo.import_history(stream.as_slice(), "fixture").unwrap();
    let before = repo.status().unwrap();
    assert!(repo.rebase(onto, 1, "merge-refused").is_err());
    assert_eq!(repo.status().unwrap(), before);
}

#[test]
fn same_path_binary_conflicts_are_authored_as_exact_signed_source_not_fake_clean_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(dir.path()).unwrap();
    let base_source = select(&mut repo, &[("f", b"\0base")], "base-source")
        .after
        .root;
    commit(&mut repo, "base");
    let onto_source = select(&mut repo, &[("f", b"\0onto")], "onto-source")
        .after
        .root;
    let onto = commit(&mut repo, "onto");
    repo.undo("onto", 4, "back").unwrap();
    let tip_source = select(&mut repo, &[("f", b"\0tip")], "tip-source")
        .after
        .root;
    let tip = commit(&mut repo, "tip");
    let replay = repo.rebase(onto, 7, "conflicted-rebase").unwrap();
    let engine = repo.load_root(replay.after.root).unwrap();
    let entry = engine.read(replay.after.root, &[b"f".to_vec()]).unwrap();
    let Entry::Conflict(id) = entry else {
        panic!("expected typed conflict")
    };
    let Object::Conflict(conflict) = engine.object(id).unwrap() else {
        panic!("expected conflict object")
    };
    let mut positive = vec![
        (
            repo.load_root(onto_source)
                .unwrap()
                .read(onto_source, &[b"f".to_vec()])
                .unwrap(),
            1,
        ),
        (
            repo.load_root(tip_source)
                .unwrap()
                .read(tip_source, &[b"f".to_vec()])
                .unwrap(),
            1,
        ),
    ];
    positive.sort_by_key(|(e, _)| e.encode().unwrap());
    assert_eq!(
        conflict,
        &Conflict {
            region: 0,
            positive,
            negative: vec![(
                repo.load_root(base_source)
                    .unwrap()
                    .read(base_source, &[b"f".to_vec()])
                    .unwrap(),
                1
            )]
        }
    );
    assert_eq!(
        repo.revision(replay.after.revision.unwrap())
            .unwrap()
            .change,
        repo.revision(tip).unwrap().change
    );
    assert!(repo.load_root(replay.after.revision.unwrap()).is_ok());
}

#[test]
fn concurrent_identical_or_distinct_requests_publish_only_the_winning_exact_closure() {
    for identical in [false, true] {
        let (dir, repo, base, onto, _) = branches();
        let before = repo.status().unwrap();
        let retained = ids(dir.path());
        drop(repo);
        let gate = Arc::new(Barrier::new(2));
        let threads: Vec<_> = (0..2)
            .map(|i| {
                let mut repo = Repository::open(dir.path()).unwrap();
                let gate = gate.clone();
                std::thread::spawn(move || {
                    gate.wait();
                    repo.rebase(
                        if identical || i == 0 { onto } else { base },
                        before.generation,
                        if identical || i == 0 { "race" } else { "other" },
                    )
                })
            })
            .collect();
        let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
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
        let repo = Repository::open(dir.path()).unwrap();
        let after = repo.status().unwrap();
        assert_eq!(after.generation, before.generation + 1);
        let expected: BTreeSet<_> = repo
            .load_root(after.revision.unwrap())
            .unwrap()
            .objects()
            .map(|(id, _)| id.to_bytes().unwrap().to_vec())
            .collect();
        let admitted = ids(dir.path());
        assert_eq!(
            admitted.difference(&retained).collect::<Vec<_>>(),
            expected.difference(&retained).collect::<Vec<_>>()
        );
    }
}

#[test]
fn denied_receipt_rolls_back_all_rewrite_objects_and_corrupt_inputs_never_gain_protection() {
    let (dir, mut repo, _, onto, _) = branches();
    let before = repo.status().unwrap();
    let retained = ids(dir.path());
    let db = rusqlite::Connection::open(dir.path().join(".avc/store.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER reject_rebase BEFORE INSERT ON operations WHEN NEW.kind = 'rebase' BEGIN SELECT RAISE(ABORT,'denied'); END;").unwrap();
    assert!(repo.rebase(onto, before.generation, "rebase").is_err());
    assert_eq!(repo.status().unwrap(), before);
    assert_eq!(ids(dir.path()), retained);
    assert!(repo.receipt("rebase").unwrap().is_none());
    db.execute_batch("DROP TRIGGER reject_rebase").unwrap();
    assert!(repo.rebase(onto, before.generation, "rebase").is_ok());
    for payload in [b"feature\n".as_slice(), b"upstream\n", b"base\n"] {
        let (dir, mut repo, _, onto, _) = branches();
        let before = repo.status().unwrap();
        let retained = ids(dir.path());
        let blob = Object::Blob(payload.to_vec()).id().unwrap();
        let db = rusqlite::Connection::open(dir.path().join(".avc/store.sqlite")).unwrap();
        db.execute_batch("DROP TRIGGER objects_no_update").unwrap();
        db.execute(
            "UPDATE objects SET payload = ?1, length = ?2 WHERE id = ?3",
            rusqlite::params![
                b"corrupted".as_slice(),
                9,
                blob.to_bytes().unwrap().as_slice()
            ],
        )
        .unwrap();
        assert!(
            repo.rebase(onto, before.generation, "corrupt-rebase")
                .is_err()
        );
        assert_eq!(repo.status().unwrap(), before);
        assert_eq!(ids(dir.path()), retained);
        assert!(repo.receipt("corrupt-rebase").unwrap().is_none());
    }
}

#[test]
fn rootless_change_replays_from_empty_and_preserves_modes_symlinks_and_empty_directories() {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(dir.path()).unwrap();
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let executable = Entry::File {
        blob: engine.blob(b"#!/bin/sh\n".to_vec()).unwrap(),
        executable: true,
    };
    let link = Entry::Symlink(engine.blob(b"../raw-target".to_vec()).unwrap());
    let binary = Entry::File {
        blob: engine.blob(vec![0, 255, 13, 10]).unwrap(),
        executable: false,
    };
    let upstream = engine
        .edit(
            empty,
            &[
                Edit {
                    path: vec![b"dir".to_vec()],
                    expected: Entry::Absent,
                    value: Entry::Directory(empty),
                },
                Edit {
                    path: vec![b"exec".to_vec()],
                    expected: Entry::Absent,
                    value: executable.clone(),
                },
            ],
        )
        .unwrap();
    repo.commit_source(&engine, upstream, 0, "onto-source")
        .unwrap();
    let onto = commit(&mut repo, "onto");
    repo.undo("onto", 2, "unborn-again").unwrap();
    let source = engine
        .edit(
            empty,
            &[
                Edit {
                    path: vec![b"link".to_vec()],
                    expected: Entry::Absent,
                    value: link.clone(),
                },
                Edit {
                    path: vec![b"bytes".to_vec()],
                    expected: Entry::Absent,
                    value: binary.clone(),
                },
            ],
        )
        .unwrap();
    repo.commit_source(&engine, source, 3, "tip-source")
        .unwrap();
    let tip = commit(&mut repo, "tip");
    assert!(repo.revision(tip).unwrap().parents.is_empty());
    let replay = repo.rebase(onto, 5, "rootless-rebase").unwrap();
    let engine = repo.load_root(replay.after.root).unwrap();
    assert_eq!(
        engine.list(replay.after.root, &[]).unwrap(),
        vec![
            (b"bytes".to_vec(), binary),
            (b"dir".to_vec(), Entry::Directory(empty)),
            (b"exec".to_vec(), executable),
            (b"link".to_vec(), link),
        ]
    );
    assert_eq!(
        repo.revision(replay.after.revision.unwrap())
            .unwrap()
            .parents,
        vec![onto]
    );
    assert_eq!(
        repo.revision(replay.after.revision.unwrap())
            .unwrap()
            .change,
        repo.revision(tip).unwrap().change
    );
}
