use avc::engine::{Edit, Engine, Limits, MergeBudget};
use avc::{Entry, Error, Id, Object};
fn p() -> Vec<Vec<u8>> {
    vec![b"f".to_vec()]
}
fn root(engine: &mut Engine, empty: Id, byte: u8) -> Id {
    let blob = engine.blob(vec![0, byte]).unwrap();
    engine
        .edit(
            empty,
            &[Edit {
                path: p(),
                expected: Entry::Absent,
                value: Entry::File {
                    blob,
                    executable: false,
                },
            }],
        )
        .unwrap()
}
#[test]
fn divergent_resolution_of_add_add_never_deletes_path() {
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let left = root(&mut engine, empty, 1);
    let right = root(&mut engine, empty, 2);
    let conflict = engine
        .merge(empty, left, right, MergeBudget::default())
        .unwrap();
    assert!(conflict.unresolved);
    assert!(
        matches!(
            engine.merge(conflict.root, left, right, MergeBudget::default()),
            Err(Error::UnresolvedBase(_))
        ),
        "different resolutions require an explicit new base/value, not cancellation to absence"
    );
    engine.verify(left, Limits::default()).unwrap();
    engine.verify(right, Limits::default()).unwrap();
}
#[test]
fn conflicting_content_preserves_independently_resolved_mode() {
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let base = root(&mut engine, empty, 0);
    let left = root(&mut engine, empty, 1);
    let right = root(&mut engine, empty, 2);
    let Entry::File { blob, .. } = engine.read(left, &p()).unwrap() else {
        panic!()
    };
    let left = engine
        .edit(
            left,
            &[Edit {
                path: p(),
                expected: Entry::File {
                    blob,
                    executable: false,
                },
                value: Entry::File {
                    blob,
                    executable: true,
                },
            }],
        )
        .unwrap();
    let merged = engine
        .merge(base, left, right, MergeBudget::default())
        .unwrap();
    let Entry::Conflict(id) = engine.read(merged.root, &p()).unwrap() else {
        panic!()
    };
    let Object::Conflict(c) = engine.object(id).unwrap() else {
        panic!()
    };
    assert!(
        c.positive.iter().chain(&c.negative).all(|(e, _)| matches!(
            e,
            Entry::File {
                executable: true,
                ..
            }
        )),
        "the clean mode result must survive either content choice"
    );
}
