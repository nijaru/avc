use avc::codec::{Entry, Error, Id, Node, Object};
use avc::engine::{Edit, Engine, Limits, Relocate};
use std::collections::BTreeMap;
use std::process::Command;

const STACK: usize = 2 * 1024 * 1024;

// An overflow aborts rather than unwinds. Keep it out of the parent test harness.
fn on_small_stack(case: &str, run: fn()) {
    if std::env::var("AVC_STACK_CASE").as_deref() == Ok(case) {
        std::thread::Builder::new()
            .stack_size(STACK)
            .spawn(run)
            .unwrap()
            .join()
            .unwrap();
        return;
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", case, "--nocapture"])
        .env("AVC_STACK_CASE", case)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{case}: {}\n{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn names() -> Vec<Vec<u8>> {
    let base = vec![b'a'; 255];
    let mut names = vec![base.clone()];
    for i in 0..510 {
        let mut name = base.clone();
        name[i / 2] = if i % 2 == 0 { b'q' } else { b'b' };
        names.push(name);
    }
    names
}

fn put(engine: &mut Engine, root: Id, path: Vec<Vec<u8>>, value: Entry) -> Id {
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

fn save(objects: &mut BTreeMap<Id, Object>, object: Object) -> Id {
    let id = object.id().unwrap();
    objects.insert(id, object);
    id
}

// Independent breadth-first range partitioning over complete sorted keys; no
// recursive builder, and no incremental Engine operations used to make imports.
fn canonical(entries: &[(Vec<u8>, Entry)], objects: &mut BTreeMap<Id, Object>) -> Id {
    enum Plan {
        Leaf(Node),
        Branch(u32, Vec<u8>, Option<Entry>, Vec<(u8, usize)>),
    }
    let mut ranges = vec![(0, entries.len(), 0)];
    let mut plans = Vec::new();
    let mut cursor = 0;
    while cursor < ranges.len() {
        let (start, end, offset) = ranges[cursor];
        if end - start <= 32 {
            plans.push(Plan::Leaf(Node::Leaf(
                entries[start..end]
                    .iter()
                    .map(|(key, entry)| (key[offset..].to_vec(), entry.clone()))
                    .collect(),
            )));
        } else {
            let shared = entries[start]
                .0
                .iter()
                .zip(&entries[end - 1].0)
                .take_while(|(a, b)| a == b)
                .count();
            let prefix = entries[start].0[offset..shared].to_vec();
            let mut first = start;
            let terminal = if entries[first].0.len() == shared {
                first += 1;
                Some(entries[start].1.clone())
            } else {
                None
            };
            let mut children = Vec::new();
            while first < end {
                let slot = entries[first].0[shared];
                let mut last = first + 1;
                while last < end && entries[last].0[shared] == slot {
                    last += 1;
                }
                children.push((slot, ranges.len()));
                ranges.push((first, last, shared + 1));
                first = last;
            }
            plans.push(Plan::Branch(
                (end - start).try_into().unwrap(),
                prefix,
                terminal,
                children,
            ));
        }
        cursor += 1;
    }
    let mut ids = BTreeMap::new();
    for (index, plan) in plans.into_iter().enumerate().rev() {
        let node = match plan {
            Plan::Leaf(node) => node,
            Plan::Branch(count, prefix, terminal, children) => Node::Branch {
                count,
                prefix,
                terminal,
                children: children
                    .into_iter()
                    .map(|(slot, index)| (slot, ids[&index]))
                    .collect(),
            },
        };
        ids.insert(index, save(objects, Object::Map(node)));
    }
    ids[&0]
}

fn fixture(wrappers: usize) -> (BTreeMap<Id, Object>, Id, Id, Entry) {
    let mut objects = BTreeMap::new();
    let blob = save(&mut objects, Object::Blob(b"same file".to_vec()));
    let file = Entry::File {
        blob,
        executable: false,
    };
    let mut entries: Vec<_> = names()
        .into_iter()
        .map(|name| {
            (
                name.iter()
                    .flat_map(|b| [b >> 4, b & 15])
                    .collect::<Vec<_>>(),
                file.clone(),
            )
        })
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let map = canonical(&entries, &mut objects);
    let mut root = map;
    for _ in 0..wrappers {
        root = wrap(&mut objects, root, b"d");
    }
    (objects, root, map, file)
}

fn wrap(objects: &mut BTreeMap<Id, Object>, child: Id, name: &[u8]) -> Id {
    save(
        objects,
        Object::Map(Node::Leaf(vec![(
            name.iter().flat_map(|b| [b >> 4, b & 15]).collect(),
            Entry::Directory(child),
        )])),
    )
}

#[test]
fn incremental_radix_on_small_stack() {
    on_small_stack("incremental_radix_on_small_stack", || {
        let mut engine = Engine::new();
        let empty = engine.empty().unwrap();
        let file = Entry::File {
            blob: engine.blob(b"same file".to_vec()).unwrap(),
            executable: false,
        };
        let mut root = empty;
        for name in names() {
            root = put(&mut engine, root, vec![name], file.clone());
        }
        let (_, expected, _, _) = fixture(0);
        assert_eq!(root, expected);
        let mut expected_names = names();
        expected_names.sort();
        assert_eq!(
            engine.list(root, &[]).unwrap(),
            expected_names
                .into_iter()
                .map(|name| (name, file.clone()))
                .collect::<Vec<_>>()
        );
        engine.verify(root, Limits::default()).unwrap();
        for name in names().into_iter().rev() {
            root = put(&mut engine, root, vec![name], Entry::Absent);
        }
        assert_eq!(root, empty);
    });
}

#[test]
fn nested_radix_on_small_stack() {
    on_small_stack("nested_radix_on_small_stack", || {
        let (objects, root, map, file) = fixture(255);
        let limits = Limits {
            objects: 2000,
            bindings: 2000,
            bytes: 1_000_000,
        };
        let mut engine = Engine::new();
        engine
            .import(
                objects.iter().map(|(id, o)| (*id, o.encode().unwrap())),
                root,
                limits,
            )
            .unwrap();
        engine.verify(root, limits).unwrap();
        let parent = vec![b"d".to_vec(); 255];
        assert_eq!(engine.read(root, &parent).unwrap(), Entry::Directory(map));
        assert_eq!(engine.list(root, &parent).unwrap().len(), 511);
        let mut path = parent.clone();
        path.push(vec![b'a'; 255]);
        let replacement = Entry::File {
            blob: engine.blob(b"changed".to_vec()).unwrap(),
            executable: true,
        };
        let changed = put(&mut engine, root, path.clone(), replacement.clone());
        let diff = engine.diff(root, changed).unwrap();
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0].path, path);
        assert_eq!(diff[0].before, file);
        assert_eq!(diff[0].after, replacement);
        assert!(engine.diff_with_limit(root, changed, 0).is_err());
        engine.verify(changed, Limits::default()).unwrap();
        assert_eq!(put(&mut engine, changed, path.clone(), file.clone()), root);
        let mut destination = parent;
        destination.push(b"other".to_vec());
        let moved = engine
            .transform(
                root,
                &[Relocate {
                    source: path.clone(),
                    destination: destination.clone(),
                    expected_source: file.clone(),
                    expected_destination: Entry::Absent,
                    copy: false,
                    overwrite: false,
                }],
                &[],
            )
            .unwrap();
        assert_eq!(engine.read(moved, &path).unwrap(), Entry::Absent);
        assert_eq!(engine.read(moved, &destination).unwrap(), file);
        assert_eq!(engine.diff(root, moved).unwrap().len(), 2);
        engine.verify(moved, Limits::default()).unwrap();

        // Exercise merge's existing diff_views caller through nested sources.
        let mut other_path = vec![b"d".to_vec(); 255];
        other_path.push(names().pop().unwrap());
        let right = put(&mut engine, root, other_path.clone(), replacement.clone());
        let merged = engine
            .merge(root, changed, right, avc::engine::MergeBudget::default())
            .unwrap();
        assert!(!merged.unresolved);
        assert_eq!(engine.read(merged.root, &path).unwrap(), replacement);
        assert_eq!(engine.read(merged.root, &other_path).unwrap(), replacement);
        engine.verify(merged.root, Limits::default()).unwrap();

        // Shared directory bounds must remain exact, and diff's path buffer
        // must unwind all the way back before visiting a sibling directory.
        let child = engine.read(root, &[b"d".to_vec()]).unwrap();
        let branched = put(&mut engine, root, vec![b"z".to_vec()], child);
        let left_change = put(&mut engine, branched, path.clone(), replacement.clone());
        let mut sibling_path = path.clone();
        sibling_path[0] = b"z".to_vec();
        let both = put(&mut engine, left_change, sibling_path.clone(), replacement);
        let differences = engine.diff(branched, both).unwrap();
        assert_eq!(
            differences.into_iter().map(|d| d.path).collect::<Vec<_>>(),
            vec![path, sibling_path]
        );
        engine.verify(both, Limits::default()).unwrap();

        // Traverse both sides of the full radix, not just one changed binding.
        let empty = engine.empty().unwrap();
        let cleared = put(
            &mut engine,
            root,
            vec![b"d".to_vec(); 255],
            Entry::Directory(empty),
        );
        let mut expected_names = names();
        expected_names.sort();
        let expected_paths = expected_names
            .into_iter()
            .map(|name| {
                let mut path = vec![b"d".to_vec(); 255];
                path.push(name);
                path
            })
            .collect::<Vec<_>>();
        for (before, after) in [(root, cleared), (cleared, root)] {
            let differences = engine.diff(before, after).unwrap();
            assert_eq!(
                differences.into_iter().map(|d| d.path).collect::<Vec<_>>(),
                expected_paths
            );
        }
        assert!(engine.diff_with_limit(root, cleared, 510).is_err());
    });
}

#[test]
fn exact_source_bounds_and_budgets_on_small_stack() {
    on_small_stack("exact_source_bounds_and_budgets_on_small_stack", || {
        let (mut objects, map, _, file) = fixture(0);
        let mut parent = vec![vec![b'd'; 14]; 254];
        parent.push(vec![b'd'; 30]);
        let mut root = map;
        for name in parent.iter().rev() {
            root = wrap(&mut objects, root, name);
        }
        let mut path = parent.clone();
        path.push(vec![b'a'; 255]);
        assert_eq!(path.len(), 256);
        assert_eq!(
            path.iter().map(Vec::len).sum::<usize>() + path.len() - 1,
            4096
        );
        let records: Vec<_> = objects
            .iter()
            .map(|(id, o)| (*id, o.encode().unwrap()))
            .collect();
        let exact = Limits {
            objects: records.len(),
            bindings: 511 + parent.len(),
            bytes: records.iter().map(|(_, bytes)| bytes.len()).sum(),
        };
        let mut engine = Engine::new();
        engine.import(records.clone(), root, exact).unwrap();
        engine.verify(root, exact).unwrap();
        let changed = put(
            &mut engine,
            root,
            path.clone(),
            Entry::Symlink(match file {
                Entry::File { blob, .. } => blob,
                _ => unreachable!(),
            }),
        );
        assert_eq!(engine.diff(root, changed).unwrap()[0].path, path);
        engine.verify(changed, Limits::default()).unwrap();
        for limits in [
            Limits {
                objects: exact.objects - 1,
                ..exact
            },
            Limits {
                bindings: exact.bindings - 1,
                ..exact
            },
            Limits {
                bytes: exact.bytes - 1,
                ..exact
            },
        ] {
            let mut rejected = Engine::new();
            assert!(matches!(
                rejected.import(records.clone(), root, limits),
                Err(Error::Limit(_))
            ));
            assert_eq!(rejected.object_count(), 0);
            assert!(matches!(engine.verify(root, limits), Err(Error::Limit(_))));
        }

        // Individually legal names/maps must still fail rooted context checks.
        // Exceed bytes by one without exceeding component depth.
        let mut overlong = map;
        parent[0].push(b'd');
        for name in parent.iter().rev() {
            overlong = wrap(&mut objects, overlong, name);
        }
        // Exceed depth by one, but stay far below the byte bound.
        let mut too_deep = map;
        for _ in 0..256 {
            too_deep = wrap(&mut objects, too_deep, b"d");
        }
        for invalid_root in [overlong, too_deep] {
            let mut rejected = Engine::new();
            assert!(matches!(
                rejected.import(
                    objects.iter().map(|(id, o)| (*id, o.encode().unwrap())),
                    invalid_root,
                    Limits::default(),
                ),
                Err(Error::Invalid(_))
            ));
            assert_eq!(rejected.object_count(), 0);
        }
    });
}
