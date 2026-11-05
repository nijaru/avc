use avc::engine::{Edit, Engine, Limits, MergeBudget, Relocate};
use avc::{Entry, Error, Id, Object};

fn path(s: &str) -> Vec<Vec<u8>> {
    s.split('/').map(|p| p.as_bytes().to_vec()).collect()
}
fn file(e: &mut Engine, s: &[u8]) -> Entry {
    Entry::File {
        blob: e.blob(s.to_vec()).unwrap(),
        executable: false,
    }
}
fn put(e: &mut Engine, r: Id, p: &str, v: Entry) -> Id {
    let expected = e.read(r, &path(p)).unwrap();
    e.edit(
        r,
        &[Edit {
            path: path(p),
            expected,
            value: v,
        }],
    )
    .unwrap()
}
fn conflict(e: &Engine, r: Id, p: &str) -> avc::Conflict {
    let Entry::Conflict(id) = e.read(r, &path(p)).unwrap() else {
        panic!("expected conflict")
    };
    let Object::Conflict(c) = e.object(id).unwrap() else {
        panic!("kind")
    };
    c.clone()
}
fn content(e: &Engine, r: Id, p: &str) -> Vec<u8> {
    let Entry::File { blob, .. } = e.read(r, &path(p)).unwrap() else {
        panic!("file")
    };
    let Object::Blob(b) = e.object(blob).unwrap() else {
        panic!("blob")
    };
    b.clone()
}

#[test]
fn two_checkout_free_agents_compose_disjoint_exact_candidates() {
    let mut e = Engine::new();
    let empty = e.empty().unwrap();
    let a = file(&mut e, b"initial-a");
    let b = file(&mut e, b"initial-b");
    let base = put(&mut e, empty, "a", a.clone());
    let base = put(&mut e, base, "b", b.clone());
    let new_a = file(&mut e, b"agent-a");
    let new_b = file(&mut e, b"agent-b");
    let left = put(&mut e, base, "a", new_a.clone());
    let right = put(&mut e, base, "b", new_b.clone());
    let composed = e.merge(base, left, right, MergeBudget::default()).unwrap();
    assert!(!composed.unresolved);
    assert_eq!(e.read(composed.root, &path("a")).unwrap(), new_a);
    assert_eq!(e.read(composed.root, &path("b")).unwrap(), new_b);
    assert_eq!(
        e.merge(base, right, left, MergeBudget::default()).unwrap(),
        composed
    );
    assert_eq!(e.read(base, &path("a")).unwrap(), a);
    assert!(matches!(
        e.edit(
            left,
            &[Edit {
                path: path("a"),
                expected: a,
                value: b
            }]
        ),
        Err(Error::Stale)
    ));
    e.verify(composed.root, Limits::default()).unwrap();
}
#[test]
fn independent_hunks_and_mode_compose_but_binary_and_overlap_remain_typed() {
    let mut e = Engine::new();
    let empty = e.empty().unwrap();
    let original = file(&mut e, b"a\r\nb\r\nc\r\n");
    let base = put(&mut e, empty, "f", original);
    let l = file(&mut e, b"A\r\nb\r\nc\r\n");
    let r = file(&mut e, b"a\r\nb\r\nC\r\n");
    let left = put(&mut e, base, "f", l);
    let right = put(&mut e, base, "f", r);
    let merged = e.merge(base, left, right, MergeBudget::default()).unwrap();
    assert_eq!(content(&e, merged.root, "f"), b"A\r\nb\r\nC\r\n");
    let Entry::File { blob, .. } = e.read(base, &path("f")).unwrap() else {
        panic!()
    };
    let mode = put(
        &mut e,
        base,
        "f",
        Entry::File {
            blob,
            executable: true,
        },
    );
    let merged = e.merge(base, left, mode, MergeBudget::default()).unwrap();
    assert!(!merged.unresolved);
    assert!(matches!(
        e.read(merged.root, &path("f")).unwrap(),
        Entry::File {
            executable: true,
            ..
        }
    ));
    assert_eq!(content(&e, merged.root, "f"), b"A\r\nb\r\nc\r\n");
    let other = file(&mut e, b"different\0");
    let right = put(&mut e, base, "f", other);
    let unresolved = e.merge(base, left, right, MergeBudget::default()).unwrap();
    assert!(unresolved.unresolved);
    let c = conflict(&e, unresolved.root, "f");
    assert_eq!(c.positive.len(), 2);
    assert_eq!(c.negative.len(), 1);
}
#[test]
fn normalized_rebase_revert_and_resolution_do_not_use_lossy_same_change_rule() {
    let mut e = Engine::new();
    let empty = e.empty().unwrap();
    let mut roots = Vec::new();
    let mut values = Vec::new();
    for x in 0..6 {
        let value = file(&mut e, &[0, x]);
        values.push(value.clone());
        roots.push(put(&mut e, empty, "f", value));
    }
    let c = e
        .merge(roots[0], roots[1], roots[2], MergeBudget::default())
        .unwrap();
    let rebased = e
        .merge(roots[2], c.root, roots[3], MergeBudget::default())
        .unwrap();
    let result = conflict(&e, rebased.root, "f");
    assert_eq!(result.negative, vec![(values[0].clone(), 1)]);
    assert_eq!(result.positive.len(), 2);
    assert!(result.positive.contains(&(values[1].clone(), 1)));
    assert!(result.positive.contains(&(values[3].clone(), 1)));
    let back = e
        .merge(roots[3], rebased.root, roots[2], MergeBudget::default())
        .unwrap();
    assert_eq!(back.root, c.root);
    // An unresolved base is not a reversible edit recipe. Divergent changes
    // from it require explicit resolution, never symbolic false-clean output.
    assert!(matches!(
        e.merge(c.root, roots[4], roots[5], MergeBudget::default()),
        Err(Error::UnresolvedBase(_))
    ));
    e.verify(c.root, Limits::default()).unwrap();
}
#[test]
fn documented_identical_change_intent_loss_is_visible_in_source_identity() {
    let mut e = Engine::new();
    let empty = e.empty().unwrap();
    let a = file(&mut e, b"old");
    let base = put(&mut e, empty, "f", a);
    let b = file(&mut e, b"new");
    let changed = put(&mut e, base, "f", b);
    // Rebase of the same result onto itself becomes empty; rebasing that empty
    // delta back cannot reconstruct the original recipe. This is the specified
    // snapshot tradeoff, not an invertible Pijul-style change algebra.
    let forward = e
        .merge(base, changed, changed, MergeBudget::default())
        .unwrap();
    assert_eq!(forward.root, changed);
    let back = e
        .merge(changed, forward.root, base, MergeBudget::default())
        .unwrap();
    assert_eq!(back.root, base);
    assert_ne!(back.root, changed);
    // Original immutable source remains available; history layer must retain it.
    assert_eq!(content(&e, changed, "f"), b"new");
}
#[test]
fn structural_delete_type_and_move_conflicts_keep_whole_values() {
    let mut e = Engine::new();
    let empty = e.empty().unwrap();
    let a = file(&mut e, b"a");
    let b = file(&mut e, b"b");
    let subtree = put(&mut e, empty, "f", a);
    let changed = put(&mut e, subtree, "f", b);
    let base = put(&mut e, empty, "dir", Entry::Directory(subtree));
    let deleted = put(&mut e, base, "dir", Entry::Absent);
    let edited = put(&mut e, base, "dir", Entry::Directory(changed));
    let merged = e
        .merge(base, deleted, edited, MergeBudget::default())
        .unwrap();
    assert!(merged.unresolved);
    let c = conflict(&e, merged.root, "dir");
    assert!(c.positive.contains(&(Entry::Absent, 1)));
    assert!(c.positive.contains(&(Entry::Directory(changed), 1)));
    assert_eq!(c.negative, vec![(Entry::Directory(subtree), 1)]);
    assert!(matches!(
        e.read(merged.root, &path("dir/f")),
        Err(Error::Conflict(_))
    ));
    let moved = e
        .transform(
            base,
            &[Relocate {
                source: path("dir"),
                destination: path("new"),
                expected_source: Entry::Directory(subtree),
                expected_destination: Entry::Absent,
                copy: false,
                overwrite: false,
            }],
            &[],
        )
        .unwrap();
    let composed = e
        .merge(base, moved, edited, MergeBudget::default())
        .unwrap();
    assert!(composed.unresolved);
    assert_eq!(
        e.read(composed.root, &path("new")).unwrap(),
        Entry::Directory(subtree)
    );
    assert!(matches!(
        e.read(composed.root, &path("dir")).unwrap(),
        Entry::Conflict(_)
    ));
    let original_file = file(&mut e, b"old type");
    let base = put(&mut e, empty, "dir", original_file);
    let left = put(&mut e, base, "dir", Entry::Directory(subtree));
    let right = put(&mut e, base, "dir", Entry::Directory(changed));
    let merged = e.merge(base, left, right, MergeBudget::default()).unwrap();
    assert!(matches!(
        e.read(merged.root, &path("dir")).unwrap(),
        Entry::Conflict(_)
    ));
    e.verify(composed.root, Limits::default()).unwrap();
}
#[test]
fn conflict_term_and_work_exhaustion_emit_no_output_root() {
    let mut e = Engine::new();
    let empty = e.empty().unwrap();
    let a = file(&mut e, &[0, 0]);
    let base = put(&mut e, empty, "f", a);
    let a = file(&mut e, &[0, 1]);
    let mut left = put(&mut e, base, "f", a);
    let mut failed = false;
    for x in 2..80 {
        let value = file(&mut e, &[0, x]);
        let right = put(&mut e, base, "f", value);
        match e.merge(base, left, right, MergeBudget::default()) {
            Ok(result) => left = result.root,
            Err(Error::Limit(_)) => {
                failed = true;
                break;
            }
            Err(other) => panic!("{other}"),
        }
    }
    assert!(failed);
    e.verify(left, Limits::default()).unwrap();
    assert!(matches!(
        e.merge(
            base,
            left,
            empty,
            MergeBudget {
                paths: 0,
                ..MergeBudget::default()
            }
        ),
        Err(Error::Limit(_))
    ));
    assert!(matches!(
        e.diff_with_limit(base, left, 0),
        Err(Error::Limit(_))
    ));
}
