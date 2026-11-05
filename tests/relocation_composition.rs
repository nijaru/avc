use avc::engine::{Edit, Engine, Limits, MergeBudget, Relocate};
use avc::{Entry, Object};
fn p(s: &str) -> Vec<Vec<u8>> {
    s.split('/').map(|s| s.as_bytes().to_vec()).collect()
}
#[test]
fn witnessed_move_edit_retains_correlated_whole_source_choices() {
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let blob = engine.blob(b"old".to_vec()).unwrap();
    let file = Entry::File {
        blob,
        executable: false,
    };
    let dir = engine
        .edit(
            empty,
            &[Edit {
                path: p("f"),
                expected: Entry::Absent,
                value: file.clone(),
            }],
        )
        .unwrap();
    let base = engine
        .edit(
            empty,
            &[Edit {
                path: p("src"),
                expected: Entry::Absent,
                value: Entry::Directory(dir),
            }],
        )
        .unwrap();
    let moved = engine
        .prepare(
            base,
            &[Relocate {
                source: p("src"),
                destination: p("lib"),
                expected_source: Entry::Directory(dir),
                expected_destination: Entry::Absent,
                copy: false,
                overwrite: false,
            }],
            &[],
        )
        .unwrap();
    let blob = engine.blob(b"new".to_vec()).unwrap();
    let edited = engine
        .prepare(
            base,
            &[],
            &[Edit {
                path: p("src/f"),
                expected: file.clone(),
                value: Entry::File {
                    blob,
                    executable: false,
                },
            }],
        )
        .unwrap();
    let composed = engine
        .compose(&moved, &edited, MergeBudget::default())
        .unwrap();
    assert!(composed.unresolved);
    assert_eq!(composed.root.kind, 4);
    let Object::Conflict(c) = engine.object(composed.root).unwrap() else {
        panic!()
    };
    assert_eq!(c.region, 1);
    assert!(c.positive.contains(&(Entry::Directory(moved.root()), 1)));
    assert!(c.positive.contains(&(Entry::Directory(edited.root()), 1)));
    assert_eq!(c.negative, vec![(Entry::Directory(base), 1)]);
    assert_eq!(
        engine
            .compose(&edited, &moved, MergeBudget::default())
            .unwrap(),
        composed
    );
    engine.verify(composed.root, Limits::default()).unwrap();
    assert!(
        matches!(
            engine.merge(
                composed.root,
                moved.root(),
                edited.root(),
                MergeBudget::default()
            ),
            Err(avc::Error::UnresolvedBase(_))
        ),
        "divergent whole-source resolutions must not resurrect the removed base"
    );
    let other = engine
        .prepare(
            base,
            &[],
            &[Edit {
                path: p("notes"),
                expected: Entry::Absent,
                value: file,
            }],
        )
        .unwrap();
    let clean = engine
        .compose(&moved, &other, MergeBudget::default())
        .unwrap();
    assert!(!clean.unresolved);
    assert_eq!(engine.read(clean.root, &p("src")).unwrap(), Entry::Absent);
    assert_eq!(
        engine.read(clean.root, &p("lib")).unwrap(),
        Entry::Directory(dir)
    );
}
