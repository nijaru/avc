#![cfg(any(target_os = "linux", target_os = "macos"))]

use avc::{
    Entry, Id, Object,
    capture::{CaptureMode, MAX_WITNESS_BYTES, observe, report},
    engine::Edit,
    engine::Engine,
};
use std::{
    ffi::OsStr,
    fs,
    os::unix::{ffi::OsStrExt, fs::PermissionsExt, fs::symlink},
    path::{Path, PathBuf},
};

fn path(names: &[&[u8]]) -> Vec<Vec<u8>> {
    names.iter().map(|name| name.to_vec()).collect()
}
fn temp() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    (temp, root)
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
fn file(engine: &mut Engine, bytes: &[u8]) -> Entry {
    Entry::File {
        blob: engine.blob(bytes.to_vec()).unwrap(),
        executable: false,
    }
}
fn payload<'a>(engine: &'a Engine, root: Id, path: &[Vec<u8>]) -> &'a [u8] {
    let id = match engine.read(root, path).unwrap() {
        Entry::File { blob, .. } | Entry::Symlink(blob) => blob,
        value => panic!("not a leaf: {value:?}"),
    };
    match engine.object(id).unwrap() {
        Object::Blob(bytes) => bytes,
        _ => panic!("not a blob"),
    }
}
fn refuses(root: &Path, paths: &[Vec<Vec<u8>>]) -> String {
    let mut engine = Engine::new();
    let base = engine.empty().unwrap();
    match observe(&mut engine, base, root, paths, CaptureMode::Leaves) {
        Ok(_) => panic!("unexpected observation success"),
        Err(error) => error.to_string(),
    }
}

#[test]
fn captures_exact_bytes_modes_links_and_nested_leaves_without_live_writes() {
    let (_temp, root) = temp();
    fs::create_dir_all(root.join("one/two")).unwrap();
    let binary = b"\0\xffhello\r\nsecond\r\n";
    fs::write(root.join("binary"), binary).unwrap();
    fs::set_permissions(root.join("binary"), fs::Permissions::from_mode(0o640)).unwrap();
    fs::write(root.join("one/two/run"), b"#!/bin/sh\r\n").unwrap();
    fs::set_permissions(root.join("one/two/run"), fs::Permissions::from_mode(0o710)).unwrap();
    fs::write(root.join("one/group-exec"), b"not owner executable").unwrap();
    fs::set_permissions(
        root.join("one/group-exec"),
        fs::Permissions::from_mode(0o610),
    )
    .unwrap();
    let target = b"../dangling-\xff\r\n";
    symlink(OsStr::from_bytes(target), root.join("link")).unwrap();
    let before = fs::metadata(root.join("binary")).unwrap();
    let mut engine = Engine::new();
    let base = engine.empty().unwrap();
    let selected = [
        path(&[b"one", b"two", b"run"]),
        path(&[b"binary"]),
        path(&[b"link"]),
        path(&[b"one", b"group-exec"]),
    ];
    let observed = observe(&mut engine, base, &root, &selected, CaptureMode::Leaves).unwrap();
    assert_eq!(payload(&engine, observed.root, &selected[1]), binary);
    assert_eq!(payload(&engine, observed.root, &selected[2]), target);
    assert!(matches!(
        engine.read(observed.root, &selected[0]).unwrap(),
        Entry::File {
            executable: true,
            ..
        }
    ));
    assert!(matches!(
        engine.read(observed.root, &selected[3]).unwrap(),
        Entry::File {
            executable: false,
            ..
        }
    ));
    assert!(observed.witnesses.starts_with(b"AVCOBS02"));
    assert!(observed.witnesses.len() <= MAX_WITNESS_BYTES);
    let report = report(&observed.witnesses).unwrap();
    assert_eq!(
        (report.files, report.symlinks, report.directories),
        (3, 1, 0)
    );
    assert_eq!(fs::read(root.join("binary")).unwrap(), binary);
    assert_eq!(
        fs::read_link(root.join("link"))
            .unwrap()
            .as_os_str()
            .as_bytes(),
        target
    );
    let after = fs::metadata(root.join("binary")).unwrap();
    assert_eq!(before.permissions().mode(), after.permissions().mode());
    assert_eq!(before.modified().unwrap(), after.modified().unwrap());
    assert!(!root.join(".avc").exists());
    fs::write(root.join("binary"), b"subsequent live edit").unwrap();
    assert_eq!(payload(&engine, observed.root, &selected[1]), binary);
}

#[cfg(target_os = "linux")]
#[test]
fn raw_non_utf8_basename_is_not_normalized() {
    let (_temp, root) = temp();
    let name = b"raw-\xff\xfe";
    fs::write(root.join(OsStr::from_bytes(name)), b"\xff\0").unwrap();
    let mut engine = Engine::new();
    let base = engine.empty().unwrap();
    let observed = observe(
        &mut engine,
        base,
        &root,
        &[path(&[name])],
        CaptureMode::Leaves,
    )
    .unwrap();
    assert_eq!(payload(&engine, observed.root, &path(&[name])), b"\xff\0");
    assert!(
        observed
            .witnesses
            .windows(name.len())
            .any(|bytes| bytes == name)
    );
}

#[test]
fn versioned_evidence_contains_exact_raw_paths_root_identity_and_leaf_stats() {
    use std::os::unix::fs::MetadataExt;
    fn take<const N: usize>(input: &mut &[u8]) -> [u8; N] {
        let (head, rest) = input.split_at(N);
        *input = rest;
        head.try_into().unwrap()
    }
    fn count(input: &mut &[u8]) -> usize {
        u32::from_le_bytes(take(input)) as usize
    }
    fn raw(input: &mut &[u8]) -> Vec<u8> {
        let length = count(input);
        let (head, rest) = input.split_at(length);
        *input = rest;
        head.to_vec()
    }
    let (_temp, root) = temp();
    let contents = b"payload-not-repeated-in-observation-metadata-0123456789";
    fs::write(root.join("file"), contents).unwrap();
    symlink(OsStr::from_bytes(b"missing-\xff"), root.join("link")).unwrap();
    let mut engine = Engine::new();
    let base = engine.empty().unwrap();
    let observation = observe(
        &mut engine,
        base,
        &root,
        &[path(&[b"link"]), path(&[b"file"])],
        CaptureMode::Leaves,
    )
    .unwrap();
    let mut input = observation.witnesses.as_slice();
    assert_eq!(&take::<8>(&mut input), b"AVCOBS02");
    assert_eq!(raw(&mut input), root.as_os_str().as_bytes());
    let chain = count(&mut input);
    assert!(chain > 0);
    for index in 0..chain {
        let stat = take::<72>(&mut input);
        if index + 1 == chain {
            let metadata = fs::metadata(&root).unwrap();
            assert_eq!(
                u64::from_le_bytes(stat[..8].try_into().unwrap()),
                metadata.dev()
            );
            assert_eq!(
                u64::from_le_bytes(stat[8..16].try_into().unwrap()),
                metadata.ino()
            );
        }
    }
    for expected in [1, 1, 0] {
        assert_eq!(u64::from_le_bytes(take(&mut input)), expected);
    }
    assert_eq!(count(&mut input), 2);
    for (name, tag) in [(b"file".as_slice(), 1), (b"link".as_slice(), 2)] {
        assert_eq!(count(&mut input), 1);
        assert_eq!(raw(&mut input), name);
        assert_eq!(take::<1>(&mut input), [tag]);
        let metadata = fs::symlink_metadata(root.join(OsStr::from_bytes(name))).unwrap();
        for expected in [metadata.dev(), metadata.ino(), u64::from(metadata.mode())] {
            assert_eq!(u64::from_le_bytes(take(&mut input)), expected);
        }
        for expected in [
            i64::try_from(metadata.size()).unwrap(),
            metadata.mtime(),
            metadata.mtime_nsec(),
            metadata.ctime(),
            metadata.ctime_nsec(),
        ] {
            assert_eq!(i64::from_le_bytes(take(&mut input)), expected);
        }
        assert_eq!(u64::from_le_bytes(take(&mut input)), metadata.nlink());
        assert_eq!(u64::from_le_bytes(take(&mut input)), metadata.size());
    }
    assert!(input.is_empty());
    assert!(
        !observation
            .witnesses
            .windows(contents.len())
            .any(|part| part == contents)
    );
}

#[test]
fn preserves_unselected_source_and_updates_only_requested_leaves() {
    let (_temp, root) = temp();
    fs::create_dir(root.join("nested")).unwrap();
    fs::write(root.join("nested/selected"), b"new selected").unwrap();
    fs::write(root.join("untracked"), b"not selected").unwrap();
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let kept = file(&mut engine, b"kept source even though absent live");
    let child = put(&mut engine, empty, path(&[b"kept"]), kept.clone());
    let old = file(&mut engine, b"old selected");
    let child = put(&mut engine, child, path(&[b"selected"]), old.clone());
    let base = put(
        &mut engine,
        empty,
        path(&[b"nested"]),
        Entry::Directory(child),
    );
    let top = file(&mut engine, b"top");
    let base = put(&mut engine, base, path(&[b"top"]), top.clone());
    let observed = observe(
        &mut engine,
        base,
        &root,
        &[path(&[b"nested", b"selected"])],
        CaptureMode::Leaves,
    )
    .unwrap();
    assert_eq!(
        engine
            .read(observed.root, &path(&[b"nested", b"kept"]))
            .unwrap(),
        kept
    );
    assert_eq!(engine.read(observed.root, &path(&[b"top"])).unwrap(), top);
    assert_eq!(
        engine.read(observed.root, &path(&[b"untracked"])).unwrap(),
        Entry::Absent
    );
    assert_eq!(
        engine.read(base, &path(&[b"nested", b"selected"])).unwrap(),
        old
    );
    assert_eq!(
        payload(&engine, observed.root, &path(&[b"nested", b"selected"])),
        b"new selected"
    );
}

#[test]
fn refuses_missing_directories_fifos_and_symlink_parents_without_escapes() {
    let (_temp, root) = temp();
    let (_outside_temp, outside) = temp();
    fs::write(outside.join("secret"), b"outside").unwrap();
    fs::create_dir(root.join("directory")).unwrap();
    symlink(&outside, root.join("parent")).unwrap();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(root.join("fifo"))
            .status()
            .unwrap()
            .success()
    );
    for selected in [
        path(&[b"missing"]),
        path(&[b"directory"]),
        path(&[b"fifo"]),
        path(&[b"parent", b"secret"]),
        path(&[b"parent", b"missing"]),
    ] {
        assert!(refuses(&root, &[selected]).contains("observation refused"));
    }
    symlink(&root, root.join("alias")).unwrap();
    assert!(refuses(&root.join("alias"), &[path(&[b"anything"])]).contains("observation refused"));
    assert_eq!(fs::read(outside.join("secret")).unwrap(), b"outside");
}

#[test]
fn validates_entire_scope_before_opening_any_leaf() {
    let (_temp, root) = temp();
    fs::write(root.join("first"), b"would otherwise capture").unwrap();
    for selected in [
        path(&[b".avc", b"objects.sqlite3"]),
        path(&[b"nested", b".git", b"config"]),
        path(&[b".git"]),
        path(&[b"..", b"escape"]),
        path(&[b"/absolute"]),
        path(&[b"nul\0"]),
        path(&[b"a/b"]),
        vec![],
    ] {
        let mut engine = Engine::new();
        let base = engine.empty().unwrap();
        let count = engine.object_count();
        let error = observe(
            &mut engine,
            base,
            &root,
            &[path(&[b"first"]), selected],
            CaptureMode::Leaves,
        );
        assert!(error.is_err());
        assert_eq!(engine.object_count(), count);
    }
    assert!(refuses(&root.join(".avc"), &[path(&[b"db"])]).contains("administrative"));
    assert!(refuses(Path::new("relative"), &[path(&[b"first"])]).contains("absolute"));
    assert!(refuses(&root.join(".."), &[path(&[b"first"])]).contains("basename"));
    let duplicate = path(&[b"first"]);
    assert!(refuses(&root, &[duplicate.clone(), duplicate]).contains("duplicate"));
    assert!(
        refuses(&root, &[path(&[b"first"]), path(&[b"first", b"child"])]).contains("overlapping")
    );
}

#[test]
fn enforces_path_count_byte_and_depth_bounds_at_public_boundary() {
    let (_temp, root) = temp();
    assert!(refuses(&root, &[]).contains("1..=1024"));
    assert!(refuses(&root, &vec![path(&[b"x"]); 1025]).contains("1..=1024"));
    let paths: Vec<_> = (0..300)
        .map(|i| vec![format!("{i:03}{}", "x".repeat(250)).into_bytes()])
        .collect();
    assert!(refuses(&root, &paths).contains("64 KiB"));
    // 257 * 255 raw basename bytes fits 64 KiB, but the required +1 for
    // each component does not. Exactly 256 such components is accepted.
    let boundary: Vec<_> = (0..257)
        .map(|i| vec![format!("{i:03}{}", "x".repeat(252)).into_bytes()])
        .collect();
    assert!(refuses(&root, &boundary).contains("64 KiB"));
    assert!(refuses(&root, &boundary[..256]).contains("lstat"));
    assert!(refuses(&root, &[vec![b"x".to_vec(); 257]]).contains("256 components"));
    let oversized_root = PathBuf::from(format!("/{}", "x".repeat(4096)));
    assert!(refuses(&oversized_root, &[path(&[b"x"])]).contains("4096"));
}

#[test]
fn oversized_sparse_leaf_refuses_before_allocating_or_storing_payload() {
    let (_temp, root) = temp();
    let huge = fs::File::create(root.join("huge")).unwrap();
    huge.set_len(8 * 1024 * 1024 * 1024).unwrap();
    let mut engine = Engine::new();
    let base = engine.empty().unwrap();
    let count = engine.object_count();
    assert!(
        observe(
            &mut engine,
            base,
            &root,
            &[path(&[b"huge"])],
            CaptureMode::Leaves
        )
        .is_err()
    );
    assert_eq!(engine.object_count(), count);
}

#[test]
fn refuses_instead_of_discarding_unselected_source_directory_children() {
    let (_temp, root) = temp();
    fs::write(root.join("directory"), b"live file").unwrap();
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let child = file(&mut engine, b"unselected child");
    let directory = put(&mut engine, empty, path(&[b"child"]), child);
    let base = put(
        &mut engine,
        empty,
        path(&[b"directory"]),
        Entry::Directory(directory),
    );
    assert!(
        observe(
            &mut engine,
            base,
            &root,
            &[path(&[b"directory"])],
            CaptureMode::Leaves
        )
        .is_err()
    );
    assert_eq!(
        payload(&engine, base, &path(&[b"directory", b"child"])),
        b"unselected child"
    );
}

#[test]
fn refuses_unreadable_file_when_permissions_are_enforced() {
    let (_temp, root) = temp();
    let selected = root.join("unreadable");
    fs::write(&selected, b"secret").unwrap();
    fs::set_permissions(&selected, fs::Permissions::from_mode(0o0)).unwrap();
    // Privileged test runners can legitimately read mode 000; don't manufacture
    // a permission error that the actual filesystem did not report.
    if fs::File::open(&selected).is_err() {
        assert!(refuses(&root, &[path(&[b"unreadable"])]).contains("open leaf"));
    }
    fs::set_permissions(&selected, fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn refuses_physical_admin_root_aliases_and_regular_file_hardlinks() {
    let (_temp, root) = temp();
    fs::create_dir_all(root.join(".avc/nested")).unwrap();
    fs::write(root.join(".avc/nested/store.sqlite"), b"administration").unwrap();
    symlink(root.join(".avc"), root.join("alias")).unwrap();
    assert!(
        refuses(&root.join("alias/nested"), &[path(&[b"store.sqlite"])]).contains("administrative")
    );
    assert!(
        refuses(&root.join(".avc/nested"), &[path(&[b"store.sqlite"])]).contains("administrative")
    );
    fs::hard_link(
        root.join(".avc/nested/store.sqlite"),
        root.join("ordinary-name"),
    )
    .unwrap();
    assert!(refuses(&root, &[path(&[b"ordinary-name"])]).contains("hardlinked"));
    // This also excludes ordinary hardlinks unrelated to administration: it is
    // an explicit narrow-subset restriction, not an attempt to identify SQLite.
    fs::write(root.join("ordinary"), b"ordinary source").unwrap();
    fs::hard_link(root.join("ordinary"), root.join("other-name")).unwrap();
    assert!(refuses(&root, &[path(&[b"ordinary"])]).contains("hardlinked"));
    assert!(
        refuses(&root, &[path(&[b".AVC", b"nested", b"store.sqlite"])]).contains("administrative")
    );
}

#[test]
fn v2_ancestor_records_fit_the_shared_witness_budget_without_repeating_every_parent() {
    let (_temp, root) = temp();
    let parents = vec![b"d".to_vec(); 20];
    let mut directory = root.clone();
    for name in &parents {
        directory.push(OsStr::from_bytes(name));
    }
    fs::create_dir_all(&directory).unwrap();
    let mut selected = Vec::new();
    for index in 0..256 {
        let name = format!("{index:03}").into_bytes();
        fs::write(directory.join(OsStr::from_bytes(&name)), b"x").unwrap();
        let mut path = parents.clone();
        path.push(name);
        selected.push(path);
    }
    let mut engine = Engine::new();
    let base = engine.empty().unwrap();
    let observed = observe(&mut engine, base, &root, &selected, CaptureMode::Leaves).unwrap();
    assert!(observed.witnesses.len() < MAX_WITNESS_BYTES);
    let report = report(&observed.witnesses).unwrap();
    assert_eq!(
        (report.files, report.symlinks, report.directories),
        (256, 0, 0)
    );
}

#[test]
fn capture_accepts_the_exact_native_blob_payload_limit_and_refuses_one_more_byte() {
    let (_temp, root) = temp();
    let file = fs::File::create(root.join("boundary")).unwrap();
    file.set_len(16 * 1024 * 1024).unwrap();
    let mut engine = Engine::new();
    let base = engine.empty().unwrap();
    let selected = path(&[b"boundary"]);
    let observed = observe(
        &mut engine,
        base,
        &root,
        std::slice::from_ref(&selected),
        CaptureMode::Leaves,
    )
    .unwrap();
    // Canonical Blob payloads are raw bytes: no hypothetical length prefix is
    // charged against their physical payload limit (codec.rs Object::encode).
    assert_eq!(
        payload(&engine, observed.root, &selected).len(),
        16 * 1024 * 1024
    );
    file.set_len(16 * 1024 * 1024 + 1).unwrap();
    assert!(refuses(&root, &[selected]).contains("limit exceeded"));
}

#[test]
fn final_closure_payload_limit_also_covers_unselected_base_objects() {
    let (_temp, root) = temp();
    fs::write(root.join("selected"), b"small").unwrap();
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let large = Entry::File {
        blob: engine.blob(vec![0; 16 * 1024 * 1024 + 1]).unwrap(),
        executable: false,
    };
    let base = put(&mut engine, empty, path(&[b"unselected"]), large);
    assert!(
        observe(
            &mut engine,
            base,
            &root,
            &[path(&[b"selected"])],
            CaptureMode::Leaves
        )
        .is_err()
    );
}

#[test]
fn recursive_builds_exact_canonical_subtree_and_updates_without_touching_outside_scope() {
    let (_temp, root) = temp();
    // APFS rejects invalid UTF-8 names and normalizes composed Unicode. Use an
    // already-decomposed spelling there, and arbitrary non-UTF-8 bytes on Linux.
    let raw_name: &[u8] = if cfg!(target_os = "linux") {
        b"raw-\xff\xfe"
    } else {
        b"raw-e\xcc\x81"
    };
    let unicode = "e\u{301}";
    fs::create_dir_all(root.join("tree").join(unicode).join("empty")).unwrap();
    for (name, contents, mode) in [
        (b"plain".as_slice(), b"new".as_slice(), 0o600),
        (b"run", b"run", 0o710),
        (b"group", b"group", 0o610),
        (raw_name, b"\0\xff", 0o600),
    ] {
        let live = root.join("tree").join(OsStr::from_bytes(name));
        fs::write(&live, contents).unwrap();
        fs::set_permissions(live, fs::Permissions::from_mode(mode)).unwrap();
    }
    let target = b"../../outside-\xff";
    symlink(OsStr::from_bytes(target), root.join("tree/link")).unwrap();
    fs::write(root.join("unselected"), b"not captured").unwrap();
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let old = file(&mut engine, b"old");
    let old_tree = put(&mut engine, empty, path(&[b"plain"]), old);
    let base = put(
        &mut engine,
        empty,
        path(&[b"tree"]),
        Entry::Directory(old_tree),
    );
    let retained = file(&mut engine, b"outside Base need not exist live");
    let base = put(&mut engine, base, path(&[b"retained"]), retained.clone());

    let mut expected_tree = empty;
    for (name, contents, executable) in [
        (b"plain".as_slice(), b"new".as_slice(), false),
        (b"run", b"run", true),
        (b"group", b"group", false),
        (raw_name, b"\0\xff", false),
    ] {
        let blob = engine.blob(contents.to_vec()).unwrap();
        expected_tree = put(
            &mut engine,
            expected_tree,
            path(&[name]),
            Entry::File { blob, executable },
        );
    }
    let blob = engine.blob(target.to_vec()).unwrap();
    expected_tree = put(
        &mut engine,
        expected_tree,
        path(&[b"link"]),
        Entry::Symlink(blob),
    );
    let nested = put(
        &mut engine,
        empty,
        path(&[b"empty"]),
        Entry::Directory(empty),
    );
    expected_tree = put(
        &mut engine,
        expected_tree,
        path(&[unicode.as_bytes()]),
        Entry::Directory(nested),
    );
    let expected = put(
        &mut engine,
        base,
        path(&[b"tree"]),
        Entry::Directory(expected_tree),
    );
    let observed = observe(
        &mut engine,
        base,
        &root,
        &[path(&[b"tree"])],
        CaptureMode::Recursive,
    )
    .unwrap();
    assert_eq!(observed.root, expected);
    let counts = report(&observed.witnesses).unwrap();
    assert_eq!(
        (counts.files, counts.symlinks, counts.directories),
        (4, 1, 3)
    );
    assert!(counts.excluded.is_empty());
    assert_eq!(
        engine.read(observed.root, &path(&[b"retained"])).unwrap(),
        retained
    );

    fs::write(root.join("tree/plain"), b"updated").unwrap();
    fs::write(root.join("tree").join(unicode).join("added"), b"added").unwrap();
    let updated = file(&mut engine, b"updated");
    let expected = put(&mut engine, expected, path(&[b"tree", b"plain"]), updated);
    let added = file(&mut engine, b"added");
    let expected = put(
        &mut engine,
        expected,
        path(&[b"tree", unicode.as_bytes(), b"added"]),
        added,
    );
    let observed = observe(
        &mut engine,
        observed.root,
        &root,
        &[path(&[b"tree"])],
        CaptureMode::Recursive,
    )
    .unwrap();
    assert_eq!(observed.root, expected);
    assert_eq!(report(&observed.witnesses).unwrap().files, 5);
}

#[test]
fn recursive_exclusions_preserve_native_admin_data_including_absent_live_bindings() {
    let (_temp, root) = temp();
    fs::create_dir_all(root.join("nested/.GiT")).unwrap();
    fs::create_dir_all(root.join(".AVC")).unwrap();
    // An unreadable administration file must not be opened, even by a scan.
    fs::write(root.join(".AVC/store.sqlite"), b"not source").unwrap();
    fs::set_permissions(
        root.join(".AVC/store.sqlite"),
        fs::Permissions::from_mode(0o0),
    )
    .unwrap();
    symlink("/never-follow-this-target", root.join("nested/.GiT/config")).unwrap();
    fs::write(root.join("ordinary"), b"source").unwrap();
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let protected = file(&mut engine, b"Native .avc binding from another source");
    let protected_child = put(&mut engine, empty, path(&[b"data"]), protected.clone());
    let base = put(
        &mut engine,
        empty,
        path(&[b".avc"]),
        Entry::Directory(protected_child),
    );
    let base = put(&mut engine, base, path(&[b".AVC"]), protected.clone());
    let base = put(&mut engine, base, path(&[b".git"]), protected.clone());
    let nested = put(&mut engine, empty, path(&[b".GiT"]), protected.clone());
    let base = put(
        &mut engine,
        base,
        path(&[b"nested"]),
        Entry::Directory(nested),
    );
    let ordinary = file(&mut engine, b"source");
    let expected = put(&mut engine, base, path(&[b"ordinary"]), ordinary);
    let observed = observe(&mut engine, base, &root, &[vec![]], CaptureMode::Recursive).unwrap();
    assert_eq!(observed.root, expected);
    let mut counts = report(&observed.witnesses).unwrap();
    assert_eq!(
        (counts.files, counts.symlinks, counts.directories),
        (1, 0, 2)
    );
    counts.excluded.sort();
    let mut excluded = vec![
        path(&[b".avc"]),
        path(&[b".AVC"]),
        path(&[b".git"]),
        path(&[b"nested", b".GiT"]),
    ];
    excluded.sort();
    assert_eq!(counts.excluded, excluded);
    assert_eq!(
        payload(&engine, observed.root, &path(&[b".avc", b"data"])),
        b"Native .avc binding from another source"
    );
    fs::set_permissions(
        root.join(".AVC/store.sqlite"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
}

#[test]
fn root_recursive_scans_empty_directories_but_never_follows_directory_symlinks() {
    let (_temp, root) = temp();
    let (_outside_temp, outside) = temp();
    fs::create_dir(root.join("empty")).unwrap();
    fs::write(outside.join("secret"), b"outside").unwrap();
    symlink(&outside, root.join("link")).unwrap();
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let expected = put(
        &mut engine,
        empty,
        path(&[b"empty"]),
        Entry::Directory(empty),
    );
    let blob = engine
        .blob(outside.as_os_str().as_bytes().to_vec())
        .unwrap();
    let expected = put(
        &mut engine,
        expected,
        path(&[b"link"]),
        Entry::Symlink(blob),
    );
    let observed = observe(&mut engine, empty, &root, &[vec![]], CaptureMode::Recursive).unwrap();
    assert_eq!(observed.root, expected);
    let counts = report(&observed.witnesses).unwrap();
    assert_eq!(
        (counts.files, counts.symlinks, counts.directories),
        (0, 1, 2)
    );
}

#[test]
fn recursive_refuses_missing_tracked_bindings_and_directory_to_leaf_or_missing() {
    let (_temp, root) = temp();
    fs::create_dir_all(root.join("selected/nested")).unwrap();
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let tracked = file(&mut engine, b"tracked");
    let nested = put(&mut engine, empty, path(&[b"missing"]), tracked);
    let selected = put(
        &mut engine,
        empty,
        path(&[b"nested"]),
        Entry::Directory(nested),
    );
    let base = put(
        &mut engine,
        empty,
        path(&[b"selected"]),
        Entry::Directory(selected),
    );
    let error = observe(
        &mut engine,
        base,
        &root,
        &[path(&[b"selected"])],
        CaptureMode::Recursive,
    )
    .err()
    .unwrap()
    .to_string();
    assert!(
        error.contains("missing previously-selected path")
            && error.contains("deletion unsupported"),
        "{error}"
    );
    assert!(error.contains(&format!(
        "{:?}",
        path(&[b"selected", b"nested", b"missing"])
    )));
    fs::remove_dir_all(root.join("selected/nested")).unwrap();
    for replacement in [Some(false), Some(true), None] {
        match replacement {
            Some(false) => fs::write(root.join("selected/nested"), b"replacement").unwrap(),
            Some(true) => symlink("target", root.join("selected/nested")).unwrap(),
            None => {}
        }
        assert!(
            observe(
                &mut engine,
                base,
                &root,
                &[path(&[b"selected"])],
                CaptureMode::Recursive
            )
            .is_err()
        );
        if replacement.is_some() {
            fs::remove_file(root.join("selected/nested")).unwrap();
        }
    }
    fs::remove_dir(root.join("selected")).unwrap();
    let error = observe(
        &mut engine,
        base,
        &root,
        &[path(&[b"selected"])],
        CaptureMode::Recursive,
    )
    .err()
    .unwrap()
    .to_string();
    assert!(error.contains("deletion unsupported"));
    fs::write(root.join("selected"), b"replacement").unwrap();
    assert!(
        observe(
            &mut engine,
            base,
            &root,
            &[path(&[b"selected"])],
            CaptureMode::Recursive
        )
        .is_err()
    );
}

#[test]
fn recursive_explicit_directory_can_replace_source_leaf_but_partial_scope_cannot() {
    let (_temp, root) = temp();
    fs::create_dir(root.join("selected")).unwrap();
    fs::write(root.join("selected/file"), b"new").unwrap();
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let old = file(&mut engine, b"old");
    let link = Entry::Symlink(engine.blob(b"target".to_vec()).unwrap());
    let value = file(&mut engine, b"new");
    let expected_child = put(&mut engine, empty, path(&[b"file"]), value);
    let expected = put(
        &mut engine,
        empty,
        path(&[b"selected"]),
        Entry::Directory(expected_child),
    );
    for prior in [Entry::Absent, old, link] {
        let base = put(&mut engine, empty, path(&[b"selected"]), prior.clone());
        if prior != Entry::Absent {
            let error = observe(
                &mut engine,
                base,
                &root,
                &[path(&[b"selected", b"file"])],
                CaptureMode::Recursive,
            )
            .err()
            .unwrap()
            .to_string();
            assert!(error.contains("source parent is not a directory"));
        }
        let observed = observe(
            &mut engine,
            base,
            &root,
            &[path(&[b"selected"])],
            CaptureMode::Recursive,
        )
        .unwrap();
        assert_eq!(observed.root, expected);
    }
}

#[test]
fn recursive_scope_preflight_refuses_overlap_root_overlap_and_admin_before_io() {
    let mut engine = Engine::new();
    let base = engine.empty().unwrap();
    for selected in [
        vec![vec![], path(&[b"other"])],
        vec![path(&[b"a"]), path(&[b"a", b"b"])],
        vec![vec![], vec![]],
        vec![path(&[b"allowed"]), path(&[b".AvC", b"db"])],
    ] {
        let before = engine.object_count();
        let error = observe(
            &mut engine,
            base,
            Path::new("/nonexistent-observer-root"),
            &selected,
            CaptureMode::Recursive,
        )
        .err()
        .unwrap()
        .to_string();
        assert!(!error.contains("canonicalize"), "{error}");
        assert_eq!(engine.object_count(), before);
    }
}

#[test]
fn recursive_expanded_path_limit_is_checked_even_for_empty_directories() {
    let (_temp, root) = temp();
    let selected = vec![b"d".to_vec(); 256];
    let mut directory = root.clone();
    for _ in 0..256 {
        directory.push("d");
    }
    fs::create_dir_all(&directory).unwrap();
    let mut engine = Engine::new();
    let base = engine.empty().unwrap();
    let observed = observe(
        &mut engine,
        base,
        &root,
        std::slice::from_ref(&selected),
        CaptureMode::Recursive,
    )
    .unwrap();
    assert_eq!(report(&observed.witnesses).unwrap().directories, 1);
    // Traverse all 257 directories rather than merely opening a deep scope.
    let whole = observe(&mut engine, base, &root, &[vec![]], CaptureMode::Recursive).unwrap();
    assert_eq!(whole.root, observed.root);
    assert_eq!(report(&whole.witnesses).unwrap().directories, 257);
    fs::create_dir(directory.join("one-too-deep")).unwrap();
    let error = observe(
        &mut engine,
        base,
        &root,
        &[selected],
        CaptureMode::Recursive,
    )
    .err()
    .unwrap()
    .to_string();
    assert!(error.contains("expanded path exceeds"), "{error}");
}

#[test]
fn recursive_refuses_conflicts_inside_scope_instead_of_erasing_alternatives() {
    let (_temp, root) = temp();
    fs::create_dir(root.join("selected")).unwrap();
    fs::write(root.join("selected/file"), b"live").unwrap();
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let left = file(&mut engine, b"left");
    let right = file(&mut engine, b"right");
    let left = put(&mut engine, empty, path(&[b"file"]), left);
    let right = put(&mut engine, empty, path(&[b"file"]), right);
    let conflicted = engine
        .merge(empty, left, right, avc::engine::MergeBudget::default())
        .unwrap();
    assert!(conflicted.unresolved);
    let base = put(
        &mut engine,
        empty,
        path(&[b"selected"]),
        Entry::Directory(conflicted.root),
    );
    let error = observe(
        &mut engine,
        base,
        &root,
        &[path(&[b"selected"])],
        CaptureMode::Recursive,
    )
    .err()
    .unwrap()
    .to_string();
    assert!(error.contains("conflict replacement is unsupported"));
    assert!(matches!(
        engine.read(base, &path(&[b"selected", b"file"])).unwrap(),
        Entry::Conflict(_)
    ));
    // Leaf mode keeps its existing explicit conflict-resolution contract.
    let observed = observe(
        &mut engine,
        base,
        &root,
        &[path(&[b"selected", b"file"])],
        CaptureMode::Leaves,
    )
    .unwrap();
    assert_eq!(
        payload(&engine, observed.root, &path(&[b"selected", b"file"])),
        b"live"
    );
}

#[test]
fn recursive_accepts_explicit_file_and_link_scopes_without_enumerating_parents() {
    let (_temp, root) = temp();
    fs::create_dir(root.join("parent")).unwrap();
    fs::write(root.join("parent/file"), b"file").unwrap();
    symlink("missing", root.join("link")).unwrap();
    fs::write(root.join("parent/unselected"), b"unselected").unwrap();
    let mut engine = Engine::new();
    let base = engine.empty().unwrap();
    let paths = [path(&[b"link"]), path(&[b"parent", b"file"])];
    let leaves = observe(&mut engine, base, &root, &paths, CaptureMode::Leaves).unwrap();
    let recursive = observe(&mut engine, base, &root, &paths, CaptureMode::Recursive).unwrap();
    assert_eq!(recursive.root, leaves.root);
    let counts = report(&recursive.witnesses).unwrap();
    assert_eq!(
        (counts.files, counts.symlinks, counts.directories),
        (1, 1, 0)
    );
    assert_eq!(
        engine
            .read(recursive.root, &path(&[b"parent", b"unselected"]))
            .unwrap(),
        Entry::Absent
    );
}

#[test]
fn report_refuses_unknown_truncated_inconsistent_or_trailing_evidence() {
    let (_temp, root) = temp();
    fs::write(root.join("file"), b"bytes").unwrap();
    let mut engine = Engine::new();
    let base = engine.empty().unwrap();
    let observed = observe(
        &mut engine,
        base,
        &root,
        &[path(&[b"file"])],
        CaptureMode::Leaves,
    )
    .unwrap();
    for length in 0..observed.witnesses.len() {
        assert!(report(&observed.witnesses[..length]).is_err());
    }
    let mut bad = observed.witnesses.clone();
    bad[7] = b'1';
    assert!(report(&bad).is_err());
    let mut bad = observed.witnesses.clone();
    bad.extend_from_slice(b"trailing");
    assert!(report(&bad).is_err());
    let mut bad = observed.witnesses.clone();
    let chain_offset = 12 + root.as_os_str().as_bytes().len();
    let chain =
        u32::from_le_bytes(bad[chain_offset..chain_offset + 4].try_into().unwrap()) as usize;
    let counts_offset = chain_offset + 4 + chain * 72;
    bad[counts_offset] = 2;
    assert!(report(&bad).is_err());
    let mut bad = observed.witnesses.clone();
    let record_offset = counts_offset + 28;
    bad[record_offset..record_offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(report(&bad).is_err());
    assert!(report(&vec![0; MAX_WITNESS_BYTES + 1]).is_err());
}
