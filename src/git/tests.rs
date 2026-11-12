use super::*;

#[test]
fn exact_budget_boundaries_do_not_mutate_on_refusal() {
    let mut budget = Budget {
        bytes: MAX_CLOSURE_BYTES - MAX_OBJECT_BYTES,
        objects: MAX_OBJECTS - 1,
    };
    budget.add(MAX_OBJECT_BYTES).unwrap();
    assert_eq!(budget.bytes, MAX_CLOSURE_BYTES);
    assert_eq!(budget.objects, MAX_OBJECTS);
    assert!(budget.add(0).is_err());
    assert!(Budget::default().add(MAX_OBJECT_BYTES + 1).is_err());
    let mut budget = Budget {
        bytes: MAX_CLOSURE_BYTES,
        objects: 0,
    };
    assert!(budget.add(1).is_err());
    assert_eq!(budget.objects, 0);
}

#[test]
fn expanded_depth_path_and_entry_limits() {
    let mut bounds = Bounds::default();
    bounds
        .entry(
            b"x",
            Bounds {
                entries: MAX_ENTRIES - 1,
                depth: 255,
                path: 4094,
            },
        )
        .unwrap();
    assert_eq!(bounds.entries, MAX_ENTRIES);
    assert_eq!(bounds.depth, 256);
    assert_eq!(bounds.path, 4096);
    assert!(Bounds::default().entry(b"x", bounds).is_err());
    assert!(
        Bounds::default()
            .entry(
                b"long",
                Bounds {
                    path: 4096,
                    depth: 1,
                    ..Bounds::default()
                }
            )
            .is_err()
    );
}

#[test]
fn commit_required_headers_utf8_and_committer_raw_metadata() {
    let oid: GitOid = "0123456789012345678901234567890123456789".parse().unwrap();
    let base = b"tree 0123456789012345678901234567890123456789\nauthor A <a> 0 +0000\ncommitter Non-UTF8 \xff <c> 1 -0000\n\n\0exact\n";
    let commit = parse_commit(oid, base.to_vec()).unwrap();
    assert_eq!(commit.raw, base);
    assert_eq!(commit.author, "A <a>");
    assert_eq!(commit.time_ns, 1_000_000_000);
    assert_eq!(commit.message.as_bytes(), b"\0exact\n");
    for header in [
        b"tree 0123456789012345678901234567890123456789".as_slice(),
        b"author A <a> 0 +0000",
        b"committer C <c> 0 +0000",
    ] {
        let mut raw = header.to_vec();
        raw.push(b'\n');
        raw.extend_from_slice(base);
        assert!(parse_commit(oid, raw).is_err());
    }
    for raw in [
        b"tree 0123456789012345678901234567890123456789\n\nm".as_slice(),
        b"author A <a> 0 +0000\ncommitter C <c> 0 +0000\n\nm",
    ] {
        assert!(parse_commit(oid, raw.to_vec()).is_err());
    }
}
