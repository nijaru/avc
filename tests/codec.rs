use std::{collections::BTreeMap, io, str::FromStr};

use avc::{
    BLOB_KIND, Conflict, DecodeLimits, Entry, Error, Id, MAP_LEAF_KIND, Node, Object,
    REVISION_KIND, Revision, validate_name, validate_path,
};

fn id(kind: u8, byte: u8) -> Id {
    Id {
        kind,
        digest: [byte; 32],
    }
}

fn file(byte: u8) -> Entry {
    Entry::File {
        blob: id(BLOB_KIND, byte),
        executable: false,
    }
}

fn directory(byte: u8) -> Entry {
    Entry::Directory(id(MAP_LEAF_KIND, byte))
}

// These byte builders deliberately do not use the production encoder/validator.
fn reference(kind: u8, byte: u8) -> Vec<u8> {
    let mut bytes = vec![kind];
    bytes.extend_from_slice(&[byte; 32]);
    bytes
}

fn descriptor(tag: u8, kind: u8, byte: u8) -> Vec<u8> {
    let mut bytes = vec![tag];
    if tag != 0 {
        bytes.extend(reference(kind, byte));
    }
    if tag == 1 {
        bytes.push(0);
    }
    bytes
}

fn nibbles(bytes: &mut Vec<u8>, suffix: &[u8]) {
    bytes.extend_from_slice(&(suffix.len() as u16).to_le_bytes());
    for pair in suffix.chunks(2) {
        bytes.push((pair[0] << 4) | pair.get(1).copied().unwrap_or(0));
    }
}

fn leaf(bindings: &[(Vec<u8>, Vec<u8>)]) -> Vec<u8> {
    let mut bytes = vec![bindings.len() as u8];
    for (suffix, entry) in bindings {
        nibbles(&mut bytes, suffix);
        bytes.extend_from_slice(entry);
    }
    bytes
}

fn branch(count: u32, prefix: &[u8], terminal: Option<&[u8]>, children: &[(u8, u8)]) -> Vec<u8> {
    let mut bytes = count.to_le_bytes().to_vec();
    nibbles(&mut bytes, prefix);
    bytes.push(u8::from(terminal.is_some()));
    if let Some(entry) = terminal {
        bytes.extend_from_slice(entry);
    }
    let bitmap = children
        .iter()
        .fold(0u16, |bits, (nibble, _)| bits | (1 << nibble));
    bytes.extend_from_slice(&bitmap.to_le_bytes());
    for (_, kind) in children {
        bytes.extend(reference(*kind, 0));
    }
    bytes
}

fn conflict(region: u8, positive: &[(u8, Vec<u8>)], negative: &[(u8, Vec<u8>)]) -> Vec<u8> {
    let mut bytes = vec![region];
    for terms in [positive, negative] {
        bytes.push(terms.len() as u8);
        for (multiplicity, entry) in terms {
            bytes.push(*multiplicity);
            bytes.extend_from_slice(entry);
        }
    }
    bytes
}

fn revision() -> Revision {
    Revision {
        root: id(MAP_LEAF_KIND, 0x12),
        parents: vec![id(REVISION_KIND, 0x34), id(REVISION_KIND, 0x56)],
        change: [0x78; 16],
        author: "Zoë <z@example.test>".into(),
        message: "message\0\r\n🦀".into(),
        time_ns: -1_234_567_890,
        timezone_minutes: -1439,
    }
}

fn raw_revision(revision: &Revision) -> Vec<u8> {
    let mut bytes = reference(revision.root.kind, revision.root.digest[0]);
    bytes.push(revision.parents.len() as u8);
    for parent in &revision.parents {
        bytes.extend(reference(parent.kind, parent.digest[0]));
    }
    bytes.extend_from_slice(&revision.change);
    for text in [&revision.author, &revision.message] {
        bytes.extend_from_slice(&(text.len() as u32).to_le_bytes());
        bytes.extend_from_slice(text.as_bytes());
    }
    bytes.extend_from_slice(&revision.time_ns.to_le_bytes());
    bytes.extend_from_slice(&revision.timezone_minutes.to_le_bytes());
    bytes
}

fn decode(kind: u8, bytes: &[u8]) -> avc::Result<Object> {
    Object::decode(Id::hash(kind, bytes)?, bytes)
}

fn rejects(kind: u8, bytes: &[u8]) {
    assert!(
        matches!(decode(kind, bytes), Err(Error::Invalid(_))),
        "accepted malformed kind {kind}: {bytes:?}"
    );
}

fn exact(object: Object, expected: Vec<u8>) {
    assert_eq!(object.encode().unwrap(), expected);
    let object_id = object.id().unwrap();
    assert_eq!(object_id.kind, object.kind());
    assert_eq!(object_id, Id::hash(object.kind(), &expected).unwrap());
    assert_eq!(Object::decode(object_id, &expected).unwrap(), object);
    if object.kind() != BLOB_KIND {
        for length in 0..expected.len() {
            rejects(object.kind(), &expected[..length]);
        }
        let mut trailing = expected;
        trailing.push(0);
        rejects(object.kind(), &trailing);
    }
}

#[test]
fn typed_hash_domain_and_streaming() {
    for payload in [vec![], b"\0raw\r\n\xff".to_vec(), vec![0xa5; 131_073]] {
        for kind in BLOB_KIND..=REVISION_KIND {
            let mut preimage = b"AVC-OBJECT\0v1\0".to_vec();
            preimage.push(kind);
            preimage.extend_from_slice(&(payload.len() as u64).to_le_bytes());
            preimage.extend_from_slice(&payload);
            let expected = Id {
                kind,
                digest: *blake3::hash(&preimage).as_bytes(),
            };
            assert_eq!(Id::hash(kind, &payload).unwrap(), expected);
            assert_eq!(
                Id::hash_reader(kind, payload.len() as u64, payload.as_slice()).unwrap(),
                expected
            );
        }
    }
    assert_ne!(
        Id::hash(1, b"same").unwrap().digest,
        Id::hash(2, b"same").unwrap().digest
    );
    assert_ne!(Id::hash(1, b"x").unwrap(), Id::hash(1, b"x\0").unwrap());
    assert!(matches!(
        Id::hash_reader(1, 2, &b"x"[..]),
        Err(Error::Io(_))
    ));
    assert!(matches!(
        Id::hash_reader(1, 0, &b"x"[..]),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        Id::hash_reader(1, 1, &b"xx"[..]),
        Err(Error::Invalid(_))
    ));

    struct Broken;
    impl io::Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("read failure"))
        }
    }
    assert!(matches!(Id::hash_reader(1, 1, Broken), Err(Error::Io(_))));
    assert!(matches!(Id::hash_reader(1, 0, Broken), Err(Error::Io(_))));
}

#[test]
fn canonical_id_text_and_wire() {
    for kind in BLOB_KIND..=REVISION_KIND {
        let object_id = id(kind, 0xab);
        let text = format!("avc1-k{kind:02x}-{}", "ab".repeat(32));
        assert_eq!(object_id.to_string(), text);
        assert_eq!(Id::from_str(&text).unwrap(), object_id);
        let wire = reference(kind, 0xab);
        assert_eq!(object_id.to_bytes().unwrap().as_slice(), wire);
        assert_eq!(Id::from_bytes(&wire).unwrap(), object_id);
    }
    let text = id(BLOB_KIND, 0xab).to_string();
    for bad in [
        text.to_uppercase(),
        text.replace("ab", "aB"),
        text.replace("k01", "k1"),
        text.replace("k01", "k00"),
        text.replace("k01", "kff"),
        text.replace("k01", "k0A"),
        text.replace("k01", "k0g"),
        format!(" {text}"),
        format!("{text}\n"),
        text[..72].into(),
        format!("avc1-k01-{}é", "a".repeat(62)),
        format!("avc1-k01-{}g", "a".repeat(63)),
        "".into(),
    ] {
        assert!(bad.parse::<Id>().is_err(), "accepted {bad:?}");
    }
    for length in [0, 1, 32, 34, 100] {
        assert!(Id::from_bytes(&vec![1; length]).is_err());
    }
    for kind in [0, 6, 255] {
        assert!(Id::hash(kind, b"").is_err());
        assert!(Id::hash_reader(kind, 0, &b""[..]).is_err());
        assert!(id(kind, 0).to_bytes().is_err());
        assert!(Id::from_bytes(&reference(kind, 0)).is_err());
        assert!(Object::decode(id(kind, 0), b"").is_err());
    }
}

#[test]
fn entries_exact_bytes_and_constraints() {
    for (entry, bytes) in [
        (Entry::Absent, vec![0]),
        (file(0x11), descriptor(1, BLOB_KIND, 0x11)),
        (
            Entry::File {
                blob: id(1, 0x22),
                executable: true,
            },
            {
                let mut bytes = descriptor(1, 1, 0x22);
                bytes[34] = 1;
                bytes
            },
        ),
        (Entry::Symlink(id(1, 0x33)), descriptor(2, 1, 0x33)),
        (directory(0x44), descriptor(3, 2, 0x44)),
        (Entry::Directory(id(3, 0x55)), descriptor(3, 3, 0x55)),
        (Entry::Conflict(id(4, 0x66)), descriptor(4, 4, 0x66)),
    ] {
        assert_eq!(entry.encode().unwrap(), bytes);
        assert_eq!(Entry::decode(&bytes).unwrap(), entry);
        for length in 0..bytes.len() {
            assert!(Entry::decode(&bytes[..length]).is_err());
        }
        let mut trailing = bytes;
        trailing.push(0);
        assert!(Entry::decode(&trailing).is_err());
    }
    for bad in 2..=255 {
        let mut bytes = descriptor(1, 1, 0);
        bytes[34] = bad;
        assert!(Entry::decode(&bytes).is_err());
    }
    for tag in 5..=255 {
        assert!(Entry::decode(&[tag]).is_err());
    }
    for tag in 1..=4 {
        for kind in 0..=6 {
            let valid = match tag {
                1 | 2 => kind == 1,
                3 => kind == 2 || kind == 3,
                4 => kind == 4,
                _ => false,
            };
            assert_eq!(Entry::decode(&descriptor(tag, kind, 0)).is_ok(), valid);
        }
    }
    for entry in [
        Entry::File {
            blob: id(2, 0),
            executable: false,
        },
        Entry::Symlink(id(5, 0)),
        Entry::Directory(id(1, 0)),
        Entry::Conflict(id(3, 0)),
    ] {
        assert!(entry.encode().is_err());
    }
}

#[test]
fn exact_blob_and_leaf_grammar() {
    exact(Object::Blob(vec![]), vec![]);
    exact(
        Object::Blob(b"\0\xff\r\nraw".to_vec()),
        b"\0\xff\r\nraw".to_vec(),
    );
    exact(Object::Map(Node::Leaf(vec![])), vec![0]);
    exact(
        Object::Map(Node::Leaf(vec![(vec![6, 1], file(0x11))])),
        leaf(&[(vec![6, 1], descriptor(1, 1, 0x11))]),
    );
    exact(
        Object::Map(Node::Leaf(vec![
            (vec![], Entry::Symlink(id(1, 3))),
            (vec![0, 15, 1], directory(2)),
        ])),
        leaf(&[
            (vec![], descriptor(2, 1, 3)),
            (vec![0, 15, 1], descriptor(3, 2, 2)),
        ]),
    );
    // Odd suffixes and empty suffixes are locally valid; ancestry determines
    // byte alignment and complete-name legality, not this codec.
    for count in [0, 1, 2, 509, 510] {
        let suffix = vec![15; count];
        let bytes = leaf(&[(suffix.clone(), descriptor(1, 1, 0))]);
        let object = Object::Map(Node::Leaf(vec![(suffix, file(0))]));
        assert_eq!(object.encode().unwrap(), bytes);
        assert_eq!(decode(2, &bytes).unwrap(), object);
    }
    let bindings = (0..32).map(|n| (vec![n / 16, n % 16], file(n))).collect();
    let object = Object::Map(Node::Leaf(bindings));
    assert_eq!(decode(2, &object.encode().unwrap()).unwrap(), object);
}

#[test]
fn leaf_rejects_count_suffix_order_padding_and_absent() {
    rejects(2, &[33]);
    rejects(2, &[255]);
    rejects(2, &[1, 255, 255]);
    rejects(2, &[1, 255, 1]); // 511 nibbles, before any allocation
    for suffixes in [
        vec![vec![1], vec![1]],
        vec![vec![2], vec![1]],
        vec![vec![1, 0], vec![1]],
    ] {
        let bindings: Vec<_> = suffixes
            .into_iter()
            .map(|suffix| (suffix, descriptor(1, 1, 0)))
            .collect();
        rejects(2, &leaf(&bindings));
    }
    for padding in 1..=15 {
        let mut bytes = leaf(&[(vec![1], descriptor(1, 1, 0))]);
        bytes[3] |= padding;
        rejects(2, &bytes);
    }
    rejects(2, &leaf(&[(vec![1, 0], vec![0])]));
    for suffix in [vec![16], vec![0; 511]] {
        assert!(
            Object::Map(Node::Leaf(vec![(suffix, file(0))]))
                .encode()
                .is_err()
        );
    }
    let bindings = (0..33).map(|n| (vec![n / 16, n % 16], file(n))).collect();
    assert!(Object::Map(Node::Leaf(bindings)).encode().is_err());
    for bindings in [
        vec![(vec![1], file(0)), (vec![1], file(1))],
        vec![(vec![], Entry::Absent)],
    ] {
        assert!(Object::Map(Node::Leaf(bindings)).encode().is_err());
    }
}

#[test]
fn exact_branch_bitmap_packing_and_local_scope() {
    let children = BTreeMap::from([(0, id(2, 0)), (15, id(3, 0))]);
    exact(
        Object::Map(Node::Branch {
            count: 33,
            prefix: vec![10, 11, 12],
            terminal: None,
            children,
        }),
        branch(33, &[10, 11, 12], None, &[(0, 2), (15, 3)]),
    );
    exact(
        Object::Map(Node::Branch {
            count: u32::MAX,
            prefix: vec![],
            terminal: Some(file(7)),
            children: BTreeMap::from([(7, id(2, 0))]),
        }),
        branch(u32::MAX, &[], Some(&descriptor(1, 1, 7)), &[(7, 2)]),
    );
    let children = (0..16).map(|n| (n, 2)).collect::<Vec<_>>();
    let bytes = branch(33, &vec![15; 510], None, &children);
    assert!(decode(3, &bytes).is_ok());
    // The verifier, not the codec, proves maximal prefix, terminal byte parity,
    // legal recovered names, child cardinality, and the authenticated count.
    assert!(decode(3, &branch(33, &[1], Some(&descriptor(1, 1, 0)), &[(0, 2)])).is_ok());
}

#[test]
fn branch_rejects_local_structure_and_reference_errors() {
    for count in [0, 1, 32] {
        rejects(3, &branch(count, &[], None, &[(0, 2), (1, 2)]));
    }
    rejects(3, &branch(33, &[], None, &[]));
    rejects(3, &branch(33, &[], None, &[(0, 2)]));
    rejects(3, &branch(33, &[], Some(&descriptor(1, 1, 0)), &[]));
    rejects(3, &branch(33, &[], Some(&[0]), &[(0, 2)]));
    for bad in [0, 1, 4, 5, 255] {
        rejects(3, &branch(33, &[], None, &[(0, bad), (1, 2)]));
    }
    let valid = branch(33, &[], None, &[(0, 2), (1, 2)]);
    for flag in 2..=255 {
        let mut bytes = valid.clone();
        bytes[6] = flag;
        rejects(3, &bytes);
    }
    let mut bad_padding = branch(33, &[1], None, &[(0, 2), (1, 2)]);
    bad_padding[6] |= 1;
    rejects(3, &bad_padding);
    let mut bad_length = valid;
    bad_length[4..6].copy_from_slice(&511u16.to_le_bytes());
    rejects(3, &bad_length);
    for (count, prefix, terminal, children) in [
        (
            32,
            vec![],
            None,
            BTreeMap::from([(0, id(2, 0)), (1, id(2, 0))]),
        ),
        (33, vec![], None, BTreeMap::from([(0, id(2, 0))])),
        (
            33,
            vec![],
            Some(Entry::Absent),
            BTreeMap::from([(0, id(2, 0))]),
        ),
        (
            33,
            vec![],
            None,
            BTreeMap::from([(0, id(2, 0)), (16, id(2, 0))]),
        ),
        (
            33,
            vec![16],
            None,
            BTreeMap::from([(0, id(2, 0)), (1, id(2, 0))]),
        ),
        (
            33,
            vec![0; 511],
            None,
            BTreeMap::from([(0, id(2, 0)), (1, id(2, 0))]),
        ),
        (
            33,
            vec![],
            None,
            BTreeMap::from([(0, id(1, 0)), (1, id(2, 0))]),
        ),
    ] {
        assert!(
            Object::Map(Node::Branch {
                count,
                prefix,
                terminal,
                children
            })
            .encode()
            .is_err()
        );
    }
}

#[test]
fn exact_conflicts_use_wire_order_not_rust_enum_order() {
    // Derived Rust order puts Absent last. Wire order requires it first.
    assert!(file(1) < Entry::Absent);
    let value = Conflict {
        region: 0,
        positive: vec![(Entry::Absent, 1), (file(1), 1)],
        negative: vec![(file(2), 1)],
    };
    exact(
        Object::Conflict(value.clone()),
        conflict(
            0,
            &[(1, vec![0]), (1, descriptor(1, 1, 1))],
            &[(1, descriptor(1, 1, 2))],
        ),
    );
    let mut wrong = value;
    wrong.positive.sort();
    assert!(Object::Conflict(wrong).encode().is_err());
    exact(
        Object::Conflict(Conflict {
            region: 1,
            positive: vec![(directory(1), 1), (directory(2), 1)],
            negative: vec![(directory(3), 1)],
        }),
        conflict(
            1,
            &[(1, descriptor(3, 2, 1)), (1, descriptor(3, 2, 2))],
            &[(1, descriptor(3, 2, 3))],
        ),
    );
    // 63 is the greatest usable total <=64 with signed sum exactly one.
    let bytes = conflict(
        0,
        &[(32, descriptor(1, 1, 1))],
        &[(31, descriptor(1, 1, 2))],
    );
    assert!(decode(4, &bytes).is_ok());
    let mut wrong = bytes;
    wrong[2] = 33; // signed sum 2, total 64
    rejects(4, &wrong);
    rejects(
        4,
        &conflict(
            0,
            &[(33, descriptor(1, 1, 1))],
            &[(32, descriptor(1, 1, 2))],
        ),
    );
}

#[test]
fn conflict_descriptor_identity_and_distinct_term_bounds() {
    // Multiplicity is encoded first, but does not participate in term ordering.
    // Mode, entry type, and typed directory kind all distinguish exact values.
    let value = Conflict {
        region: 0,
        positive: vec![
            (Entry::Absent, 1),
            (file(1), 2),
            (
                Entry::File {
                    blob: id(1, 1),
                    executable: true,
                },
                1,
            ),
            (Entry::Symlink(id(1, 1)), 1),
            (directory(1), 1),
            (Entry::Directory(id(3, 1)), 1),
        ],
        negative: vec![(file(9), 6)],
    };
    let mut executable = descriptor(1, 1, 1);
    executable[34] = 1;
    exact(
        Object::Conflict(value),
        conflict(
            0,
            &[
                (1, vec![0]),
                (2, descriptor(1, 1, 1)),
                (1, executable),
                (1, descriptor(2, 1, 1)),
                (1, descriptor(3, 2, 1)),
                (1, descriptor(3, 3, 1)),
            ],
            &[(6, descriptor(1, 1, 9))],
        ),
    );
    let value = Object::Conflict(Conflict {
        region: 0,
        positive: (0..32).map(|n| (file(n), 1)).collect(),
        negative: (32..63).map(|n| (file(n), 1)).collect(),
    });
    assert_eq!(decode(4, &value.encode().unwrap()).unwrap(), value);
    // A missing source root has no meaning; empty source uses an empty map.
    let value = Object::Conflict(Conflict {
        region: 1,
        positive: vec![(directory(1), 2)],
        negative: vec![(Entry::Absent, 1)],
    });
    assert!(value.encode().is_err());
    rejects(
        4,
        &conflict(1, &[(2, descriptor(3, 2, 1))], &[(1, vec![0])]),
    );
    rejects(
        4,
        &conflict(
            1,
            &[(1, vec![0]), (1, descriptor(3, 2, 1))],
            &[(1, descriptor(3, 2, 2))],
        ),
    );
}

#[test]
fn conflicts_reject_every_local_noncanonical_form() {
    let a = descriptor(1, 1, 1);
    let b = descriptor(1, 1, 2);
    let c = descriptor(1, 1, 3);
    for bytes in [
        conflict(2, &[(2, a.clone())], &[(1, b.clone())]),
        conflict(0, &[], &[]),
        conflict(0, &[(1, a.clone())], &[]),
        conflict(1, &[(1, vec![0])], &[]),
        conflict(0, &[(2, a.clone())], &[]),
        conflict(0, &[(0, a.clone())], &[]),
        conflict(0, &[(65, a.clone())], &[(64, b.clone())]),
        conflict(0, &[(2, a.clone())], &[(1, a.clone())]),
        conflict(0, &[(1, a.clone()), (1, a.clone())], &[(1, b.clone())]),
        conflict(0, &[(1, b.clone()), (1, a.clone())], &[(1, c.clone())]),
        conflict(0, &[(3, c.clone())], &[(1, b.clone()), (1, a.clone())]),
        conflict(0, &[(3, c.clone())], &[(1, a.clone()), (1, a.clone())]),
        conflict(0, &[(2, descriptor(4, 4, 1))], &[(1, b.clone())]),
        conflict(0, &[(2, a.clone())], &[(1, descriptor(4, 4, 1))]),
        conflict(1, &[(2, a.clone())], &[(1, descriptor(3, 2, 0))]),
        conflict(1, &[(2, descriptor(3, 2, 0))], &[(1, a.clone())]),
        conflict(0, &[(64, a.clone()), (64, b.clone())], &[(1, c.clone())]),
        conflict(0, &[(1, a.clone())], &[(0, b.clone())]),
        vec![0, 65],
        vec![0, 255],
        {
            let mut bytes = conflict(0, &[(2, a.clone())], &[]);
            *bytes.last_mut().unwrap() = 65; // negative distinct count
            bytes
        },
    ] {
        rejects(4, &bytes);
    }
    for value in [
        Conflict {
            region: 2,
            positive: vec![(file(1), 2)],
            negative: vec![(file(2), 1)],
        },
        Conflict {
            region: 0,
            positive: vec![],
            negative: vec![],
        },
        Conflict {
            region: 0,
            positive: vec![(file(1), 1)],
            negative: vec![],
        },
        Conflict {
            region: 0,
            positive: vec![(file(1), 1); 65],
            negative: vec![],
        },
        Conflict {
            region: 0,
            positive: vec![(file(1), 2)],
            negative: vec![(file(2), 1); 65],
        },
    ] {
        assert!(Object::Conflict(value).encode().is_err());
    }
}

#[test]
fn exact_revision_fields_and_parent_order() {
    let revision = revision();
    exact(Object::Revision(revision.clone()), raw_revision(&revision));
    let original_id = Object::Revision(revision.clone()).id().unwrap();
    let mut swapped = revision.clone();
    swapped.parents.swap(0, 1);
    assert_ne!(Object::Revision(swapped).id().unwrap(), original_id);
    for root in [id(2, 1), id(3, 1), id(4, 1)] {
        let mut boundary = revision.clone();
        boundary.root = root;
        boundary.parents = (0..16).map(|n| id(5, n)).collect();
        boundary.author = "é".repeat(2048); // cap counts bytes, not scalar values
        boundary.message = "m".repeat(1024 * 1024);
        boundary.time_ns = i64::MIN;
        boundary.timezone_minutes = 1439;
        let bytes = Object::Revision(boundary.clone()).encode().unwrap();
        assert_eq!(decode(5, &bytes).unwrap(), Object::Revision(boundary));
    }
    let mut zero = revision;
    zero.parents.clear();
    zero.author.clear();
    zero.message.clear();
    zero.change = [0; 16]; // entropy of authored ChangeIds is not a local codec property
    zero.time_ns = i64::MAX;
    zero.timezone_minutes = 0;
    exact(Object::Revision(zero.clone()), raw_revision(&zero));
}

#[test]
fn revision_rejects_caps_types_duplicates_utf8_and_timezone() {
    let base = revision();
    for root_kind in [0, 1, 5, 255] {
        let mut bad = base.clone();
        bad.root.kind = root_kind;
        assert!(Object::Revision(bad.clone()).encode().is_err());
        rejects(5, &raw_revision(&bad));
    }
    for parent_kind in [0, 1, 2, 3, 4, 255] {
        let mut bad = base.clone();
        bad.parents[0].kind = parent_kind;
        assert!(Object::Revision(bad.clone()).encode().is_err());
        rejects(5, &raw_revision(&bad));
    }
    let mut duplicate = base.clone();
    duplicate.parents.push(duplicate.parents[0]);
    let mut too_many = base.clone();
    too_many.parents = (0..17).map(|n| id(5, n)).collect();
    let mut long_author = base.clone();
    long_author.author = "a".repeat(4097);
    let mut long_message = base.clone();
    long_message.message = "m".repeat(1024 * 1024 + 1);
    for bad in [duplicate, too_many, long_author, long_message] {
        assert!(Object::Revision(bad.clone()).encode().is_err());
        rejects(5, &raw_revision(&bad));
    }
    for timezone in [i16::MIN, -1440, 1440, i16::MAX] {
        let mut bad = base.clone();
        bad.timezone_minutes = timezone;
        assert!(Object::Revision(bad.clone()).encode().is_err());
        rejects(5, &raw_revision(&bad));
    }
    let bytes = raw_revision(&base);
    let author_length_offset = 33 + 1 + 33 * base.parents.len() + 16;
    let message_length_offset = author_length_offset + 4 + base.author.len();
    for offset in [author_length_offset, message_length_offset] {
        let mut bad_length = bytes.clone();
        bad_length[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        rejects(5, &bad_length);
        for invalid_utf8 in [0xff, 0x80, 0xc0] {
            let mut bad_utf8 = bytes.clone();
            bad_utf8[offset + 4] = invalid_utf8;
            rejects(5, &bad_utf8);
        }
    }
    let mut bad_count = bytes;
    bad_count[33] = 255;
    rejects(5, &bad_count);
}

#[test]
fn hash_mismatch_and_decode_resource_limits() {
    for object in [
        Object::Blob(vec![1, 2, 3]),
        Object::Map(Node::Leaf(vec![])),
        Object::Revision(revision()),
    ] {
        let bytes = object.encode().unwrap();
        let mut wrong_hash = object.id().unwrap();
        wrong_hash.digest[0] ^= 1;
        assert!(matches!(
            Object::decode(wrong_hash, &bytes),
            Err(Error::Invalid(_))
        ));
        let limit = DecodeLimits {
            max_payload_bytes: bytes.len() - 1,
        };
        assert!(matches!(
            Object::decode_with_limits(object.id().unwrap(), &bytes, limit),
            Err(Error::Limit(_))
        ));
        let exact_limit = DecodeLimits {
            max_payload_bytes: bytes.len(),
        };
        assert_eq!(
            Object::decode_with_limits(object.id().unwrap(), &bytes, exact_limit).unwrap(),
            object
        );
    }
    let blob = vec![0; DecodeLimits::default().max_payload_bytes + 1];
    let blob_id = Id::hash(1, &blob).unwrap();
    assert!(matches!(
        Object::decode(blob_id, &blob),
        Err(Error::Limit(_))
    ));
    assert!(
        Object::decode_with_limits(
            blob_id,
            &blob,
            DecodeLimits {
                max_payload_bytes: blob.len()
            }
        )
        .is_ok()
    );
}

#[test]
fn source_names_and_paths_are_raw_bytes_with_exact_bounds() {
    for good in [
        b"a".as_slice(),
        b"...",
        b"\\",
        b"\xff\xfe",
        b"a\n",
        &[1; 255],
    ] {
        assert!(validate_name(good).is_ok());
    }
    for bad in [
        b"".as_slice(),
        b".",
        b"..",
        b"a/b",
        b"\0",
        b"a\0b",
        &[1; 256],
    ] {
        assert!(validate_name(bad).is_err());
    }
    assert!(validate_path::<&[u8]>(&[]).is_ok());
    assert!(validate_path(&[b"a"; 256]).is_ok());
    assert!(validate_path(&[b"a"; 257]).is_err());
    let mut path = vec![vec![b'a'; 255]; 16]; // 4095 bytes including 15 separators
    assert!(validate_path(&path).is_ok());
    path.push(vec![b'a']); // 4097
    assert!(validate_path(&path).is_err());
    path[0].pop(); // 4096
    assert!(validate_path(&path).is_ok());
    path[0] = b"..".to_vec();
    assert!(validate_path(&path).is_err());
    assert!(validate_path(&[b"a/b".as_slice()]).is_err());
}

#[test]
fn deterministic_bounded_malformed_payload_smoke_test() {
    // Dependency-free deterministic corpus: decoding must not panic or accept
    // bytes that cannot re-encode exactly. This is not a substitute for fuzzing.
    let mut state = 0x9e37_79b9u32;
    for length in 0..512 {
        let mut payload = Vec::with_capacity(length);
        for _ in 0..length {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            payload.push(state as u8);
        }
        for kind in 2..=5 {
            if let Ok(object) = decode(kind, &payload) {
                assert_eq!(object.encode().unwrap(), payload);
            }
        }
    }
}
