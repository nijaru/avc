//! Canonical radix encoder from sorted byte keys; no Python/JS calls or layouts.
use crate::wire::{Encode, Entry, Id, Result, Writer, name_valid};
use std::collections::BTreeMap;

pub type Store = BTreeMap<Id, Vec<u8>>;
pub type FlatMap = BTreeMap<Vec<u8>, Entry>;
struct Key<'a> {
    nibbles: Vec<u8>,
    value: &'a Entry,
}

pub fn put(store: &mut Store, kind: u8, payload: Vec<u8>) -> Result<Id> {
    let id = Id::hash(kind, &payload);
    if let Some(old) = store.get(&id) {
        if old != &payload {
            return Err("hash collision".into());
        }
    } else {
        store.insert(id, payload);
    }
    Ok(id)
}

pub fn build(values: &FlatMap, store: &mut Store) -> Result<Id> {
    let mut keys = Vec::new();
    for (name, value) in values {
        if !name_valid(name) {
            return Err("invalid map name".into());
        }
        keys.push(Key {
            nibbles: name.iter().flat_map(|b| [b >> 4, b & 15]).collect(),
            value,
        });
    }
    build_range(&keys, 0, store)
}

fn build_range(keys: &[Key<'_>], consumed: usize, store: &mut Store) -> Result<Id> {
    let mut leaf = Writer::default();
    leaf.count(keys.len())?;
    for key in keys {
        leaf.nibbles(&key.nibbles[consumed..])?;
        key.value.write(&mut leaf)?;
    }
    if leaf.0.len() <= 4096 {
        return put(store, 2, leaf.0);
    }

    let first = keys.first().ok_or("empty map cannot overflow leaf")?;
    let mut shared = first.nibbles.len() - consumed;
    for key in &keys[1..] {
        shared = first.nibbles[consumed..consumed + shared]
            .iter()
            .zip(&key.nibbles[consumed..])
            .take_while(|(a, b)| a == b)
            .count();
    }
    let end_prefix = consumed + shared;
    let terminal = first.nibbles.len() == end_prefix;
    let mut children = Vec::new();
    let mut start = usize::from(terminal);
    while start < keys.len() {
        let slot = keys[start].nibbles[end_prefix];
        let mut end = start + 1;
        while end < keys.len() && keys[end].nibbles.get(end_prefix) == Some(&slot) {
            end += 1;
        }
        children.push((slot, build_range(&keys[start..end], end_prefix + 1, store)?));
        start = end;
    }
    if children.len() + usize::from(terminal) < 2 {
        return Err("branch did not split".into());
    }
    let mut branch = Writer::default();
    branch.nibbles(&first.nibbles[consumed..end_prefix])?;
    branch.u8(u8::from(terminal));
    if terminal {
        first.value.write(&mut branch)?;
    }
    branch.u16(
        children
            .iter()
            .fold(0, |mask, (slot, _)| mask | (1 << slot)),
    );
    for (_, id) in children {
        id.write(&mut branch)?;
    }
    put(store, 3, branch.0)
}
