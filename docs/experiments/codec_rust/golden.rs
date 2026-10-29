//! Construct the pinned examples from typed values, not decoded golden bytes.
use crate::maps::{self, FlatMap, Store};
use crate::verify::Verifier;
use crate::wire::{
    Edit, Encode, Entry, Id, MergeInputs, Object, Occurrence, Record, Result, Structural, unhex,
};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Default)]
struct Fixture {
    store: Store,
    roots: BTreeMap<&'static str, Id>,
}
impl Fixture {
    fn save(&mut self, object: Object) -> Result<Id> {
        maps::put(&mut self.store, object.kind(), object.encoded()?)
    }
    fn blob(&mut self, bytes: &[u8]) -> Result<Id> {
        self.save(Object::Blob(bytes.to_vec()))
    }
    fn map(&mut self, values: &[(&[u8], Entry)]) -> Result<Id> {
        let flat: FlatMap = values
            .iter()
            .map(|(name, value)| (name.to_vec(), value.clone()))
            .collect();
        maps::build(&flat, &mut self.store)
    }
    fn state(&mut self, root: Entry) -> Result<Id> {
        self.save(Object::State(root))
    }
    fn provenance(
        &mut self,
        result: Id,
        inputs: Vec<Id>,
        occurrences: Vec<Occurrence>,
        record: Record,
    ) -> Result<Id> {
        self.save(Object::Provenance {
            result,
            inputs,
            occurrences,
            records: vec![record],
        })
    }
    fn conflict(
        &mut self,
        region: u8,
        base: Option<Entry>,
        mut alternatives: [Option<Entry>; 2],
    ) -> Result<Id> {
        if alternatives[0].encoded()? > alternatives[1].encoded()? {
            alternatives.swap(0, 1);
        }
        self.save(Object::Conflict {
            region,
            base,
            alternatives,
        })
    }
}
fn samples() -> Result<Fixture> {
    let mut b = Fixture::default();
    let file = Entry::File(b.blob(b"hello\n")?, false);
    let binary = Entry::File(b.blob(b"hello\0\xff\r\n")?, false);
    let executable = Entry::File(b.blob(b"#!/bin/sh\necho hi\n")?, true);
    let symlink = Entry::Symlink(b.blob(b"../target\0bytes")?);
    let empty_map = b.map(&[])?;
    let directory = Entry::Directory(empty_map);
    let empty = b.state(directory.clone())?;
    let single_map = b.map(&[(b"foo", file.clone())])?;
    let single = b.state(Entry::Directory(single_map))?;
    let mixed_map = b.map(&[
        (b"a", file.clone()),
        (b"ab", executable.clone()),
        (b"foo", binary.clone()),
        (b"link", symlink),
        (b"\xff", file.clone()),
    ])?;
    let mixed = b.state(Entry::Directory(mixed_map))?;
    let mut wide = FlatMap::new();
    for i in 0..130 {
        wide.insert(vec![b'a', 0x30 + i / 16, 0x30 + i % 16], file.clone());
    }
    let odd = maps::build(&wide, &mut b.store)?;
    let odd_state = b.state(Entry::Directory(odd))?;
    wide.insert(b"a".to_vec(), file.clone());
    let terminal = maps::build(&wide, &mut b.store)?;
    let terminal_state = b.state(Entry::Directory(terminal))?;
    let mut threshold = FlatMap::new();
    for i in 0..100 {
        threshold.insert(format!("k{i:02}").into_bytes(), file.clone());
    }
    threshold.insert(b"long00002".to_vec(), file.clone());
    for (label, name) in [
        ("leaf_4095", b"long0001".as_slice()),
        ("leaf_4096", b"long00001"),
        ("branch_4097", b"long000001"),
    ] {
        threshold.insert(name.to_vec(), file.clone());
        let root = maps::build(&threshold, &mut b.store)?;
        threshold.remove(name);
        b.roots.insert(label, root);
    }
    let child = b.map(&[(b"file", executable.clone())])?;
    let old_map = b.map(&[
        (b"src", Entry::Directory(child)),
        (b"untouched", file.clone()),
    ])?;
    let old = b.state(Entry::Directory(old_map))?;
    let new_map = b.map(&[
        (b"lib", Entry::Directory(child)),
        (b"untouched", file.clone()),
    ])?;
    let new = b.state(Entry::Directory(new_map))?;
    let mut revisions = Vec::new();
    for author in [b"author".as_slice(), b"different"] {
        revisions.push(b.save(Object::Revision {
            state: mixed,
            parents: vec![],
            change: [b'X'; 16],
            author: author.to_vec(),
            description: b"change".to_vec(),
            time: 123456789,
            provenance: None,
        })?);
    }
    let revision = revisions[0];
    let alternate_author = revisions[1];
    let path_conflict = b.conflict(0, Some(file.clone()), [None, Some(binary)])?;
    let path_map = b.map(&[(b"broken", Entry::Conflict(path_conflict))])?;
    let path_state = b.state(Entry::Directory(path_map))?;
    let left = Entry::Directory(b.map(&[(b"left", file.clone()), (b"same", file.clone())])?);
    let right = Entry::Directory(b.map(&[(b"right", executable), (b"same", file)])?);
    let root_conflict = b.conflict(1, Some(directory), [Some(left), Some(right)])?;
    let root_state = b.state(Entry::Conflict(root_conflict))?;
    let provenance = b.provenance(
        path_state,
        revisions.clone(),
        vec![Occurrence {
            path: vec![b"broken".to_vec()],
            conflict: path_conflict,
            roles: vec![0, 1],
        }],
        Record::Transform {
            base: empty,
            structural: vec![],
            edits: vec![Edit {
                path: vec![b"broken".to_vec()],
                expected: None,
                value: Some(Entry::Conflict(path_conflict)),
            }],
        },
    )?;
    let move_provenance = b.provenance(
        new,
        vec![],
        vec![],
        Record::Transform {
            base: old,
            structural: vec![Structural::Move(
                vec![b"src".to_vec()],
                vec![b"lib".to_vec()],
                Entry::Directory(child),
                None,
            )],
            edits: vec![],
        },
    )?;
    let observation = b.provenance(
        mixed,
        vec![revision],
        vec![],
        Record::Observation {
            base: mixed,
            revision: Some(revision),
            generation: 7,
            consistency: 0,
            receipt: Some([b'C'; 16]),
            partial: false,
            coverage_mode: 0,
            scope: vec![vec![]],
            coverage: [
                vec![
                    vec![b"a".to_vec()],
                    vec![b"ab".to_vec()],
                    vec![b"foo".to_vec()],
                    vec![b"link".to_vec()],
                    vec![vec![0xff]],
                ],
                vec![],
                vec![],
            ],
            resolutions: vec![],
        },
    )?;
    let history_merge = b.provenance(
        mixed,
        revisions.clone(),
        vec![],
        Record::Merge {
            inputs: MergeInputs::History(0, 1, true),
            algorithms: [(1, 1), (1, 1)],
            moves: [None, None],
        },
    )?;
    let source_merge = b.provenance(
        mixed,
        vec![],
        vec![],
        Record::Merge {
            inputs: MergeInputs::Source([empty, mixed, mixed]),
            algorithms: [(1, 1), (1, 1)],
            moves: [None, None],
        },
    )?;
    let merge_revision = b.save(Object::Revision {
        state: path_state,
        parents: revisions,
        change: [b'Y'; 16],
        author: b"author".to_vec(),
        description: b"merge".to_vec(),
        time: 1234567890,
        provenance: Some(provenance),
    })?;
    b.roots.extend([
        ("empty_map", empty_map),
        ("empty_state", empty),
        ("single_file", single),
        ("mixed_types", mixed),
        ("odd_nibble_branch", odd),
        ("odd_nibble_state", odd_state),
        ("terminal_branch", terminal),
        ("terminal_state", terminal_state),
        ("moved_before", old),
        ("moved_after", new),
        ("revision", revision),
        ("alternate_author_revision", alternate_author),
        ("path_conflict", path_conflict),
        ("path_conflict_state", path_state),
        ("root_coupled_conflict", root_conflict),
        ("root_conflict_state", root_state),
        ("provenance", provenance),
        ("merge_revision", merge_revision),
        ("move_provenance", move_provenance),
        ("observation_provenance", observation),
        ("history_merge_provenance", history_merge),
        ("source_merge_provenance", source_merge),
    ]);
    Ok(b)
}

#[derive(Deserialize)]
struct Expected {
    kind: u8,
    id: String,
    payload_len: usize,
    payload_hex: Option<String>,
}
pub fn check() -> Result<(usize, usize)> {
    let expected: BTreeMap<String, Expected> =
        serde_json::from_str(include_str!("../codec_vectors.json")).map_err(|e| e.to_string())?;
    let built = samples()?;
    if built.roots.len() != expected.len() {
        return Err("golden label count".into());
    }
    for (label, root) in &built.roots {
        let want = expected.get(*label).ok_or("unknown golden label")?;
        let payload = built.store.get(root).ok_or("missing constructed root")?;
        if root.kind != want.kind || root.display() != want.id || payload.len() != want.payload_len
        {
            return Err(format!("golden mismatch: {label}"));
        }
        if let Some(hex) = &want.payload_hex
            && *payload != unhex(hex.as_bytes())?
        {
            return Err(format!("golden payload mismatch: {label}"));
        }
        Verifier::check(&built.store, *root)?;
    }
    Ok((built.roots.len(), built.store.len()))
}
