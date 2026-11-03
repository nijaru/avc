//! Core, pure AVC source-object construction and validation.
//!
//! This library implements the proposed v0 byte codec and in-memory Merkle
//! construction. It does **not** durably admit source, own workspace state, or
//! provide repository operations. SQLite custody and the CLI are later slices.
pub mod merkle;
pub mod objects;
pub mod verify;

#[cfg(test)]
mod tests {
    use super::merkle::{self, FlatMap, Store};
    use super::objects::{Encode, Entry, Id, Object};
    use super::verify::Verifier;

    #[test]
    fn pinned_empty_state_and_stream_hash_match_reference() {
        let mut store = Store::new();
        let leaf = merkle::put(&mut store, 2, vec![0; 4]).unwrap();
        assert_eq!(
            leaf.display(),
            "avc1-b3-k02-3jc2jl6s7rrrbhdwslqnijidn7z73ly43l7yzcpk37rilrouqvoa"
        );
        let state_bytes = Object::State(Entry::Directory(leaf)).encoded().unwrap();
        let state = merkle::put(&mut store, 4, state_bytes).unwrap();
        assert_eq!(
            state.display(),
            "avc1-b3-k04-vmnwhh4bsvdgr3igkr7ku35zbdkwkuwugwansvrb6vo3wniyjhmq"
        );
        Verifier::check(&store, state).unwrap();
        let bytes = b"streamed source";
        let mut cursor = std::io::Cursor::new(bytes);
        assert_eq!(
            Id::hash_reader(1, bytes.len() as u64, &mut cursor).unwrap(),
            Id::hash(1, bytes)
        );
        assert!(Id::hash_reader(1, 8, &mut std::io::Cursor::new(b"short")).is_err());
    }

    #[test]
    fn directory_build_is_order_independent_and_checks_wide_branch() {
        let mut forward = FlatMap::new();
        let mut reverse = FlatMap::new();
        let blob = Id::hash(1, b"same content");
        let names: Vec<Vec<u8>> = (0..250)
            .map(|n| format!("file-{n:04}.rs").into_bytes())
            .collect();
        for name in &names {
            forward.insert(name.clone(), Entry::File(blob, false));
        }
        for name in names.iter().rev() {
            reverse.insert(name.clone(), Entry::File(blob, false));
        }
        let mut store_a = Store::new();
        let mut store_b = Store::new();
        merkle::put(&mut store_a, 1, b"same content".to_vec()).unwrap();
        merkle::put(&mut store_b, 1, b"same content".to_vec()).unwrap();
        let map_a = merkle::build(&forward, &mut store_a).unwrap();
        let map_b = merkle::build(&reverse, &mut store_b).unwrap();
        assert_eq!(map_a, map_b);
        assert_eq!(store_a, store_b);
        let state = merkle::put(
            &mut store_a,
            4,
            Object::State(Entry::Directory(map_a)).encoded().unwrap(),
        )
        .unwrap();
        Verifier::check(&store_a, state).unwrap();
    }

    #[test]
    fn incomplete_and_corrupt_closures_are_not_verified() {
        let mut store = Store::new();
        let blob = merkle::put(&mut store, 1, b"protected".to_vec()).unwrap();
        let mut directory = FlatMap::new();
        directory.insert(b"file".to_vec(), Entry::File(blob, false));
        let map = merkle::build(&directory, &mut store).unwrap();
        let state = merkle::put(
            &mut store,
            4,
            Object::State(Entry::Directory(map)).encoded().unwrap(),
        )
        .unwrap();
        assert!(Verifier::check(&store, state).is_ok());
        store.remove(&blob);
        assert!(Verifier::check(&store, state).is_err());
        store.insert(blob, b"tampered".to_vec());
        assert!(Verifier::check(&store, state).is_err());
    }
    #[test]
    fn deep_revision_ancestry_has_no_recursive_history_limit() {
        let mut store = Store::new();
        let leaf = merkle::put(&mut store, 2, vec![0; 4]).unwrap();
        let state = merkle::put(
            &mut store,
            4,
            Object::State(Entry::Directory(leaf)).encoded().unwrap(),
        )
        .unwrap();
        let mut tip = None;
        let mut oldest = None;
        for i in 0..600 {
            let payload = Object::Revision {
                state,
                parents: tip.into_iter().collect(),
                change: [i as u8; 16],
                author: b"agent".to_vec(),
                description: Vec::new(),
                time: i64::from(i),
                provenance: None,
            }
            .encoded()
            .unwrap();
            let id = merkle::put(&mut store, 5, payload).unwrap();
            oldest.get_or_insert(id);
            tip = Some(id);
        }
        let tip = tip.unwrap();
        Verifier::check(&store, tip).unwrap();
        store.remove(&oldest.unwrap());
        assert!(Verifier::check(&store, tip).is_err());
    }
}
