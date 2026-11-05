use avc::codec::{Entry, Error, Id, Node, Object};
use avc::engine::{Edit, Engine, Limits, Relocate};
use std::collections::BTreeMap;

fn path(text: &str) -> Vec<Vec<u8>> {
    text.split('/').map(|s| s.as_bytes().to_vec()).collect()
}
fn put(engine: &mut Engine, root: Id, name: &str, value: Entry) -> Id {
    let path = path(name);
    let expected = engine.read(root, &path).unwrap();
    engine
        .edit(
            root,
            &[Edit {
                path,
                expected,
                value,
            }],
        )
        .unwrap()
}
fn file(engine: &mut Engine, bytes: &[u8]) -> Entry {
    Entry::File {
        blob: engine.blob(bytes.to_vec()).unwrap(),
        executable: false,
    }
}
fn records(engine: &Engine) -> Vec<(Id, Vec<u8>)> {
    engine
        .objects()
        .map(|(id, object)| (id, object.encode().unwrap()))
        .collect()
}
fn nibble(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().flat_map(|b| [b >> 4, b & 15]).collect()
}
// Deliberately different full-map oracle: rebuild from the logical sorted set.
fn oracle(entries: &[(Vec<u8>, Entry)], objects: &mut BTreeMap<Id, Object>) -> Id {
    let node = if entries.len() <= 32 {
        Node::Leaf(entries.to_vec())
    } else {
        let first = &entries[0].0;
        let last = &entries.last().unwrap().0;
        let shared = first.iter().zip(last).take_while(|(a, b)| a == b).count();
        let mut terminal = None;
        let mut partitions: BTreeMap<u8, Vec<(Vec<u8>, Entry)>> = BTreeMap::new();
        for (key, entry) in entries {
            if key.len() == shared {
                terminal = Some(entry.clone());
            } else {
                partitions
                    .entry(key[shared])
                    .or_default()
                    .push((key[shared + 1..].to_vec(), entry.clone()));
            }
        }
        let children = partitions
            .into_iter()
            .map(|(slot, entries)| (slot, oracle(&entries, objects)))
            .collect();
        Node::Branch {
            count: entries.len() as u32,
            prefix: first[..shared].to_vec(),
            terminal,
            children,
        }
    };
    let object = Object::Map(node);
    let id = object.id().unwrap();
    objects.insert(id, object);
    id
}

#[test]
fn randomized_incremental_maps_match_rebuild_oracle_and_restore() {
    let mut engine = Engine::new();
    let mut root = engine.empty().unwrap();
    let a = file(&mut engine, b"a");
    let b = file(&mut engine, b"b");
    let mut flat = BTreeMap::new();
    let mut seed = 0x4af789321u64;
    for step in 0..2500 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let name = format!("shared-prefix-{:03}", (seed >> 32) % 350).into_bytes();
        let value = match seed % 4 {
            0 => Entry::Absent,
            1 => a.clone(),
            _ => b.clone(),
        };
        let expected = flat.get(&name).cloned().unwrap_or(Entry::Absent);
        root = engine
            .edit(
                root,
                &[Edit {
                    path: vec![name.clone()],
                    expected,
                    value: value.clone(),
                }],
            )
            .unwrap();
        if value == Entry::Absent {
            flat.remove(&name);
        } else {
            flat.insert(name, value);
        }
        let items = flat
            .iter()
            .map(|(k, v)| (nibble(k), v.clone()))
            .collect::<Vec<_>>();
        assert_eq!(root, oracle(&items, &mut BTreeMap::new()), "step {step}");
        if step % 100 == 0 {
            engine.verify(root, Limits::default()).unwrap();
        }
    }
    let mut restored = Engine::new();
    restored
        .import(records(&engine), root, Limits::default())
        .unwrap();
    assert!(restored.diff(root, root).unwrap().is_empty());
    for (name, value) in flat {
        assert_eq!(restored.read(root, &[name]).unwrap(), value);
    }
}

#[test]
fn split_collapse_prefix_terminal_and_long_keys_are_canonical() {
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let value = file(&mut engine, b"constant");
    let mut forward = empty;
    let mut reverse = empty;
    let mut names = vec!["a".to_owned(), "a".repeat(255)];
    names.extend((0..100).map(|i| format!("a{:03}", i)));
    for name in &names {
        forward = put(&mut engine, forward, name, value.clone());
    }
    for name in names.iter().rev() {
        reverse = put(&mut engine, reverse, name, value.clone());
    }
    assert_eq!(forward, reverse);
    engine.verify(forward, Limits::default()).unwrap();
    for name in &names {
        forward = put(&mut engine, forward, name, Entry::Absent);
    }
    assert_eq!(forward, empty);
}

#[test]
fn exact_witnesses_batch_atomicity_and_non_directory_obstructions() {
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let a = file(&mut engine, b"a");
    let root = put(&mut engine, empty, "a", a.clone());
    let error = engine.edit(
        root,
        &[
            Edit {
                path: path("new"),
                expected: Entry::Absent,
                value: a.clone(),
            },
            Edit {
                path: path("a"),
                expected: Entry::Absent,
                value: a.clone(),
            },
        ],
    );
    assert!(matches!(error, Err(Error::Stale)));
    assert_eq!(engine.read(root, &path("new")).unwrap(), Entry::Absent);
    assert!(
        engine
            .edit(
                root,
                &[Edit {
                    path: path("a/child"),
                    expected: Entry::Absent,
                    value: a.clone()
                }]
            )
            .is_err()
    );
    assert!(
        engine
            .edit(
                root,
                &[
                    Edit {
                        path: path("new"),
                        expected: Entry::Absent,
                        value: Entry::Directory(empty)
                    },
                    Edit {
                        path: path("new/child"),
                        expected: Entry::Absent,
                        value: a.clone()
                    }
                ]
            )
            .is_err()
    );
    let same = put(&mut engine, root, "a", a);
    assert_eq!(same, root);
}

#[test]
fn simultaneous_swap_copy_and_moves_preserve_subtrees() {
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let a = file(&mut engine, b"a");
    let b = file(&mut engine, b"b");
    let subtree = put(&mut engine, empty, "child", a.clone());
    let root = put(&mut engine, empty, "a", Entry::Directory(subtree));
    let root = put(&mut engine, root, "b", b.clone());
    let swapped = engine
        .transform(
            root,
            &[
                Relocate {
                    source: path("a"),
                    destination: path("b"),
                    expected_source: Entry::Directory(subtree),
                    expected_destination: b.clone(),
                    copy: false,
                    overwrite: true,
                },
                Relocate {
                    source: path("b"),
                    destination: path("a"),
                    expected_source: b.clone(),
                    expected_destination: Entry::Directory(subtree),
                    copy: false,
                    overwrite: true,
                },
            ],
            &[],
        )
        .unwrap();
    assert_eq!(engine.read(swapped, &path("b/child")).unwrap(), a);
    assert_eq!(engine.read(swapped, &path("a")).unwrap(), b);
    let copied = engine
        .transform(
            swapped,
            &[Relocate {
                source: path("b"),
                destination: path("c"),
                expected_source: Entry::Directory(subtree),
                expected_destination: Entry::Absent,
                copy: true,
                overwrite: false,
            }],
            &[],
        )
        .unwrap();
    assert_eq!(
        engine.read(copied, &path("c")).unwrap(),
        Entry::Directory(subtree)
    );
    assert_eq!(
        engine.read(copied, &path("b")).unwrap(),
        Entry::Directory(subtree)
    );
    assert_eq!(engine.diff(swapped, copied).unwrap().len(), 1);
    engine.verify(copied, Limits::default()).unwrap();
}

#[test]
fn moving_reused_directory_checks_descendant_path_bounds() {
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let value = file(&mut engine, b"x");
    let mut subtree = put(&mut engine, empty, &"x".repeat(255), value);
    for _ in 0..14 {
        subtree = put(
            &mut engine,
            empty,
            &"d".repeat(255),
            Entry::Directory(subtree),
        );
    }
    let root = put(&mut engine, empty, "s", Entry::Directory(subtree));
    let root = put(&mut engine, root, &"p".repeat(255), Entry::Directory(empty));
    let long = vec![vec![b'p'; 255], vec![b'd'; 255]];
    assert!(
        engine
            .transform(
                root,
                &[Relocate {
                    source: path("s"),
                    destination: long,
                    expected_source: Entry::Directory(subtree),
                    expected_destination: Entry::Absent,
                    copy: false,
                    overwrite: false
                }],
                &[]
            )
            .is_err()
    );
    assert_eq!(
        engine.read(root, &path("s")).unwrap(),
        Entry::Directory(subtree)
    );
}

#[test]
fn merkle_diff_handles_threshold_changes_and_skips_equal_source() {
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let a = file(&mut engine, b"a");
    let b = file(&mut engine, b"b");
    let mut root = empty;
    for i in 0..3000 {
        root = put(&mut engine, root, &format!("file-{i:05}"), a.clone());
    }
    let start = engine.object_count();
    let next = put(&mut engine, root, "file-02000", b.clone());
    assert!(
        engine.object_count() - start < 12,
        "one-file path-copy must not rewrite directory"
    );
    let changes = engine.diff(root, next).unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].path, path("file-02000"));
    let mut small = empty;
    for i in 0..32 {
        small = put(&mut engine, small, &format!("a{i}"), a.clone());
    }
    let branch = put(&mut engine, small, "z", a.clone());
    assert_eq!(engine.diff(small, branch).unwrap().len(), 1);
    assert!(engine.diff(root, root).unwrap().is_empty());
}

#[test]
fn full_verification_rejects_forged_counts_missing_closure_and_odd_context() {
    let mut engine = Engine::new();
    let root = engine.empty().unwrap();
    let value = file(&mut engine, b"x");
    let root = put(&mut engine, root, "a", value);
    let original = records(&engine);
    let blob = match engine.read(root, &path("a")).unwrap() {
        Entry::File { blob, .. } => blob,
        _ => unreachable!(),
    };
    let mut target = Engine::new();
    assert!(matches!(
        target.import(
            original.iter().filter(|(id, _)| *id != blob).cloned(),
            root,
            Limits::default()
        ),
        Err(Error::NeedObjects(_))
    ));
    assert_eq!(target.object_count(), 0);
    let object = Object::Map(Node::Leaf(vec![(
        vec![6],
        Entry::Directory(engine.empty().unwrap()),
    )]));
    if let Ok(id) = object.id() {
        let mut bad = records(&engine);
        bad.push((id, object.encode().unwrap()));
        assert!(target.import(bad, id, Limits::default()).is_err());
    }
    let mut wide = engine.empty().unwrap();
    for i in 0..33 {
        let file = file(&mut engine, b"x");
        wide = put(&mut engine, wide, &format!("f{i}"), file);
    }
    let Object::Map(Node::Branch {
        prefix,
        terminal,
        children,
        ..
    }) = engine.object(wide).unwrap().clone()
    else {
        panic!("branch")
    };
    let forged = Object::Map(Node::Branch {
        count: 100,
        prefix,
        terminal,
        children,
    });
    let id = forged.id().unwrap();
    let mut bad = records(&engine);
    bad.push((id, forged.encode().unwrap()));
    assert!(target.import(bad, id, Limits::default()).is_err());
    assert!(
        target
            .import(
                original,
                root,
                Limits {
                    objects: 1,
                    ..Limits::default()
                }
            )
            .is_err()
    );
}
