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
    assert!(observed.witnesses.starts_with(b"AVCOBS03"));
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
    assert_eq!(&take::<8>(&mut input), b"AVCOBS03");
    assert_eq!(take::<1>(&mut input), [0]);
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
    for expected in [1, 1, 0, 0] {
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
        let mut engine = Engine::new();
        let empty = engine.empty().unwrap();
        let old = file(&mut engine, b"old");
        let base = put(&mut engine, empty, path(&[b"unreadable"]), old.clone());
        let base = put(&mut engine, base, path(&[b"missing"]), old);
        assert!(
            observe(
                &mut engine,
                base,
                &root,
                &[vec![]],
                CaptureMode::RecursiveRemovingMissing
            )
            .is_err()
        );
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
fn ancestor_records_fit_the_shared_witness_budget_without_repeating_every_parent() {
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
fn removing_missing_prunes_only_old_children_under_a_scanned_existing_directory() {
    let (_temp, root) = temp();
    fs::create_dir_all(root.join("selected/live")).unwrap();
    fs::write(root.join("selected/live/keep"), b"new").unwrap();
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let old = file(&mut engine, b"old immutable bytes");
    let nested = put(&mut engine, empty, path(&[b"child"]), old.clone());
    let mut selected = put(&mut engine, empty, path(&[b"gone-file"]), old.clone());
    selected = put(
        &mut engine,
        selected,
        path(&[b"gone-empty"]),
        Entry::Directory(empty),
    );
    selected = put(
        &mut engine,
        selected,
        path(&[b"gone-tree"]),
        Entry::Directory(nested),
    );
    selected = put(
        &mut engine,
        selected,
        path(&[b"live"]),
        Entry::Directory(nested),
    );
    let base = put(
        &mut engine,
        empty,
        path(&[b"selected"]),
        Entry::Directory(selected),
    );
    let base = put(&mut engine, base, path(&[b"outside"]), old.clone());
    // The new allowance is explicit: neither existing mode infers absence.
    for mode in [CaptureMode::Leaves, CaptureMode::Recursive] {
        assert!(observe(&mut engine, base, &root, &[path(&[b"selected"])], mode).is_err());
    }
    let observed = observe(
        &mut engine,
        base,
        &root,
        &[path(&[b"selected"])],
        CaptureMode::RecursiveRemovingMissing,
    )
    .unwrap();
    let expected_removed = vec![
        path(&[b"selected", b"gone-empty"]),
        path(&[b"selected", b"gone-file"]),
        path(&[b"selected", b"gone-tree"]),
        path(&[b"selected", b"live", b"child"]),
    ];
    assert_eq!(
        report(&observed.witnesses).unwrap().removed,
        expected_removed
    );
    for missing in &expected_removed {
        assert_eq!(engine.read(observed.root, missing).unwrap(), Entry::Absent);
    }
    assert_eq!(
        engine.read(observed.root, &path(&[b"outside"])).unwrap(),
        old
    );
    assert_eq!(
        payload(
            &engine,
            observed.root,
            &path(&[b"selected", b"live", b"keep"])
        ),
        b"new"
    );
    assert_eq!(
        payload(&engine, base, &path(&[b"selected", b"gone-file"])),
        b"old immutable bytes"
    );
}

#[test]
fn removing_missing_empty_root_preserves_excluded_base_bindings_even_without_live_names() {
    let (_temp, root) = temp();
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let protected = file(&mut engine, b"excluded immutable bytes");
    let mut base = empty;
    for name in [b".AvC".as_slice(), b".gIT", b"gone"] {
        base = put(&mut engine, base, path(&[name]), protected.clone());
    }
    let observed = observe(
        &mut engine,
        base,
        &root,
        &[vec![]],
        CaptureMode::RecursiveRemovingMissing,
    )
    .unwrap();
    let counts = report(&observed.witnesses).unwrap();
    assert_eq!(
        (counts.files, counts.symlinks, counts.directories),
        (0, 0, 1)
    );
    assert_eq!(counts.removed, vec![path(&[b"gone"])]);
    assert_eq!(counts.excluded, vec![path(&[b".AvC"]), path(&[b".gIT"])]);
    for name in [b".AvC".as_slice(), b".gIT"] {
        assert_eq!(
            engine.read(observed.root, &path(&[name])).unwrap(),
            protected
        );
    }
    assert_eq!(engine.list(observed.root, &[]).unwrap().len(), 2);
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
}

#[test]
fn removing_missing_refuses_protected_bindings_at_any_depth_of_a_missing_directory() {
    let (_temp, root) = temp();
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let value = file(&mut engine, b"protected");
    for admin in [b".avc".as_slice(), b".GIT"] {
        let protected = put(&mut engine, empty, path(&[admin]), value.clone());
        let middle = put(
            &mut engine,
            empty,
            path(&[b"deep"]),
            Entry::Directory(protected),
        );
        let base = put(
            &mut engine,
            empty,
            path(&[b"missing"]),
            Entry::Directory(middle),
        );
        let error = observe(
            &mut engine,
            base,
            &root,
            &[vec![]],
            CaptureMode::RecursiveRemovingMissing,
        )
        .err()
        .unwrap()
        .to_string();
        assert!(
            error.contains("protected administrative binding"),
            "{error}"
        );
        assert_eq!(
            payload(&engine, base, &path(&[b"missing", b"deep", admin])),
            b"protected"
        );
    }
}

#[test]
fn removing_missing_refuses_missing_conflicts_and_directories_containing_conflicts() {
    let (_temp, root) = temp();
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let left = file(&mut engine, b"left");
    let right = file(&mut engine, b"right");
    let left = put(&mut engine, empty, path(&[b"alternative"]), left);
    let right = put(&mut engine, empty, path(&[b"alternative"]), right);
    let merged = engine
        .merge(empty, left, right, avc::engine::MergeBudget::default())
        .unwrap();
    let conflict = engine.read(merged.root, &path(&[b"alternative"])).unwrap();
    assert!(matches!(conflict, Entry::Conflict(_)));
    let deep = put(
        &mut engine,
        empty,
        path(&[b"deep"]),
        Entry::Directory(merged.root),
    );
    for prior in [conflict, Entry::Directory(deep)] {
        let base = put(&mut engine, empty, path(&[b"missing"]), prior.clone());
        let error = observe(
            &mut engine,
            base,
            &root,
            &[vec![]],
            CaptureMode::RecursiveRemovingMissing,
        )
        .err()
        .unwrap()
        .to_string();
        assert!(
            error.contains("conflict alternatives are protected"),
            "{error}"
        );
        assert_eq!(engine.read(base, &path(&[b"missing"])).unwrap(), prior);
    }
}

#[test]
fn removing_missing_still_refuses_explicit_missing_scopes_parents_and_directory_to_leaf() {
    let (_temp, root) = temp();
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let old = file(&mut engine, b"old");
    let child = put(&mut engine, empty, path(&[b"child"]), old);
    let base = put(
        &mut engine,
        empty,
        path(&[b"selected"]),
        Entry::Directory(child),
    );
    for mode in [
        CaptureMode::Leaves,
        CaptureMode::Recursive,
        CaptureMode::RecursiveRemovingMissing,
    ] {
        for scope in [
            path(&[b"selected"]),
            path(&[b"selected", b"child"]),
            path(&[b"untracked"]),
        ] {
            assert!(observe(&mut engine, base, &root, &[scope], mode).is_err());
        }
    }
    let error = observe(
        &mut engine,
        base,
        &root,
        &[path(&[b"selected"])],
        CaptureMode::RecursiveRemovingMissing,
    )
    .err()
    .unwrap()
    .to_string();
    assert!(error.contains("select an existing parent"), "{error}");
    fs::create_dir(root.join("selected")).unwrap();
    let nested_base = put(
        &mut engine,
        child,
        path(&[b"child"]),
        Entry::Directory(child),
    );
    let nested_base = put(
        &mut engine,
        empty,
        path(&[b"selected"]),
        Entry::Directory(nested_base),
    );
    for is_link in [false, true] {
        for live in [root.join("selected/child"), root.join("top")] {
            if is_link {
                symlink("dangling", &live).unwrap();
            } else {
                fs::write(&live, b"replacement").unwrap();
            }
        }
        let top_base = put(&mut engine, empty, path(&[b"top"]), Entry::Directory(child));
        for (base, scope) in [
            (nested_base, path(&[b"selected"])),
            (top_base, path(&[b"top"])),
        ] {
            assert!(
                observe(
                    &mut engine,
                    base,
                    &root,
                    &[scope],
                    CaptureMode::RecursiveRemovingMissing
                )
                .is_err()
            );
        }
        fs::remove_file(root.join("selected/child")).unwrap();
        fs::remove_file(root.join("top")).unwrap();
    }
}

#[test]
fn removing_missing_supports_leaf_to_directory_but_not_recursive_conflict_replacement() {
    let (_temp, root) = temp();
    fs::create_dir(root.join("selected")).unwrap();
    fs::write(root.join("selected/file"), b"live").unwrap();
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let left = file(&mut engine, b"left");
    let right = file(&mut engine, b"right");
    let left_root = put(&mut engine, empty, path(&[b"selected"]), left.clone());
    let right_root = put(&mut engine, empty, path(&[b"selected"]), right);
    let merged = engine
        .merge(
            empty,
            left_root,
            right_root,
            avc::engine::MergeBudget::default(),
        )
        .unwrap();
    let conflict = engine.read(merged.root, &path(&[b"selected"])).unwrap();
    let with_conflict_child = put(&mut engine, empty, path(&[b"file"]), conflict);
    let conflicted_child = put(
        &mut engine,
        empty,
        path(&[b"selected"]),
        Entry::Directory(with_conflict_child),
    );
    for mode in [
        CaptureMode::Recursive,
        CaptureMode::RecursiveRemovingMissing,
    ] {
        for base in [merged.root, conflicted_child] {
            let error = observe(&mut engine, base, &root, &[path(&[b"selected"])], mode)
                .err()
                .unwrap()
                .to_string();
            assert!(
                error.contains("recursive conflict replacement is unsupported"),
                "{error}"
            );
        }
    }
    let observed = observe(
        &mut engine,
        left_root,
        &root,
        &[path(&[b"selected"])],
        CaptureMode::RecursiveRemovingMissing,
    )
    .unwrap();
    assert_eq!(
        payload(&engine, observed.root, &path(&[b"selected", b"file"])),
        b"live"
    );
    assert!(report(&observed.witnesses).unwrap().removed.is_empty());
    assert_eq!(engine.read(left_root, &path(&[b"selected"])).unwrap(), left);
}

#[test]
fn removing_missing_uses_raw_case_renamed_directory_names_not_os_lookup_aliases() {
    let (_temp, root) = temp();
    fs::create_dir(root.join("Name")).unwrap();
    fs::write(root.join("Name/child"), b"bytes").unwrap();
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let before = observe(&mut engine, empty, &root, &[vec![]], CaptureMode::Recursive).unwrap();
    fs::rename(root.join("Name"), root.join("name")).unwrap();
    assert_eq!(
        fs::read_dir(&root)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .file_name()
            .as_bytes(),
        b"name"
    );
    let observed = observe(
        &mut engine,
        before.root,
        &root,
        &[vec![]],
        CaptureMode::RecursiveRemovingMissing,
    )
    .unwrap();
    assert_eq!(
        report(&observed.witnesses).unwrap().removed,
        vec![path(&[b"Name"])]
    );
    assert_eq!(
        engine.read(observed.root, &path(&[b"Name"])).unwrap(),
        Entry::Absent
    );
    assert_eq!(
        payload(&engine, observed.root, &path(&[b"name", b"child"])),
        b"bytes"
    );
    assert_eq!(
        payload(&engine, before.root, &path(&[b"Name", b"child"])),
        b"bytes"
    );
}

#[test]
fn removing_missing_uses_raw_nonascii_names_without_banning_normalization_aliases() {
    let (_temp, root) = temp();
    fs::create_dir(root.join("e\u{301}")).unwrap();
    let actual = fs::read_dir(&root)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .file_name();
    let actual = actual.as_bytes();
    let old = if actual == "é".as_bytes() {
        "e\u{301}".as_bytes()
    } else {
        "é".as_bytes()
    };
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let base = put(&mut engine, empty, path(&[old]), Entry::Directory(empty));
    let observed = observe(
        &mut engine,
        base,
        &root,
        &[vec![]],
        CaptureMode::RecursiveRemovingMissing,
    )
    .unwrap();
    assert_eq!(
        report(&observed.witnesses).unwrap().removed,
        vec![path(&[old])]
    );
    assert_eq!(
        engine.read(observed.root, &path(&[old])).unwrap(),
        Entry::Absent
    );
    assert_eq!(
        engine.read(observed.root, &path(&[actual])).unwrap(),
        Entry::Directory(empty)
    );
    assert_eq!(
        engine.read(base, &path(&[old])).unwrap(),
        Entry::Directory(empty)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn removing_missing_preserves_raw_binary_directory_names_and_records_exact_old_bytes() {
    let (_temp, root) = temp();
    let old = b"old-\xff";
    let new = b"new-\xfe";
    fs::create_dir(root.join(OsStr::from_bytes(new))).unwrap();
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let base = put(&mut engine, empty, path(&[old]), Entry::Directory(empty));
    let observed = observe(
        &mut engine,
        base,
        &root,
        &[vec![]],
        CaptureMode::RecursiveRemovingMissing,
    )
    .unwrap();
    assert_eq!(
        report(&observed.witnesses).unwrap().removed,
        vec![path(&[old])]
    );
    assert_eq!(
        engine.read(observed.root, &path(&[new])).unwrap(),
        Entry::Directory(empty)
    );
    assert_eq!(
        engine.read(base, &path(&[old])).unwrap(),
        Entry::Directory(empty)
    );
}

#[test]
fn removing_missing_never_turns_unsupported_or_hardlinked_live_children_into_absence() {
    let (_temp, root) = temp();
    fs::create_dir(root.join("selected")).unwrap();
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let old = file(&mut engine, b"old");
    let selected = put(&mut engine, empty, path(&[b"live"]), old.clone());
    let selected = put(&mut engine, selected, path(&[b"missing"]), old.clone());
    let base = put(
        &mut engine,
        empty,
        path(&[b"selected"]),
        Entry::Directory(selected),
    );
    assert!(
        std::process::Command::new("mkfifo")
            .arg(root.join("selected/live"))
            .status()
            .unwrap()
            .success()
    );
    assert!(
        observe(
            &mut engine,
            base,
            &root,
            &[path(&[b"selected"])],
            CaptureMode::RecursiveRemovingMissing
        )
        .is_err()
    );
    fs::remove_file(root.join("selected/live")).unwrap();
    fs::write(root.join("original"), b"bytes").unwrap();
    fs::hard_link(root.join("original"), root.join("selected/live")).unwrap();
    let error = observe(
        &mut engine,
        base,
        &root,
        &[path(&[b"selected"])],
        CaptureMode::RecursiveRemovingMissing,
    )
    .err()
    .unwrap()
    .to_string();
    assert!(error.contains("hardlinked"), "{error}");
    assert_eq!(
        engine
            .read(base, &path(&[b"selected", b"missing"]))
            .unwrap(),
        old
    );
}

// Independent v3 fixtures keep malformed-evidence expectations separate from
// the production writer. This stat describes an ordinary directory.
fn evidence_directory_stat() -> Vec<u8> {
    [1u64, 2, 0o040755, 0, 1, 0, 1, 0, 2]
        .into_iter()
        .flat_map(u64::to_le_bytes)
        .collect()
}
fn evidence_raw(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
}
fn evidence_record(names: &[&[u8]], kind: u8) -> Vec<u8> {
    let mut out = (names.len() as u32).to_le_bytes().to_vec();
    for name in names {
        evidence_raw(&mut out, name);
    }
    out.push(kind);
    out
}
fn evidence_scanned(path: &[&[u8]], names: &[&[u8]]) -> Vec<u8> {
    let mut out = evidence_record(path, 3);
    out.extend_from_slice(&evidence_directory_stat());
    out.extend_from_slice(&(names.len() as u32).to_le_bytes());
    for name in names {
        evidence_raw(&mut out, name);
    }
    out
}
fn evidence_fixture(mode: u8, counts: [u64; 4], records: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"AVCOBS03".to_vec();
    out.push(mode);
    evidence_raw(&mut out, b"/");
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&evidence_directory_stat());
    for count in counts {
        out.extend_from_slice(&count.to_le_bytes());
    }
    out.extend_from_slice(&(records.len() as u32).to_le_bytes());
    for record in records {
        out.extend_from_slice(record);
    }
    out
}

#[test]
fn v3_scanned_record_contains_complete_sorted_raw_names_including_live_exclusions() {
    let (_temp, root) = temp();
    fs::create_dir(root.join(".avc")).unwrap();
    fs::create_dir(root.join("empty")).unwrap();
    fs::write(root.join("z"), b"bytes").unwrap();
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let old = file(&mut engine, b"old");
    let base = put(&mut engine, empty, path(&[b".git"]), old.clone());
    let base = put(&mut engine, base, path(&[b"gone"]), old);
    let observed = observe(
        &mut engine,
        base,
        &root,
        &[vec![]],
        CaptureMode::RecursiveRemovingMissing,
    )
    .unwrap();
    let bytes = &observed.witnesses;
    assert_eq!(&bytes[..9], b"AVCOBS03\x02");
    let root_length = u32::from_le_bytes(bytes[9..13].try_into().unwrap()) as usize;
    assert_eq!(&bytes[13..13 + root_length], root.as_os_str().as_bytes());
    let chain_offset = 13 + root_length;
    let chain =
        u32::from_le_bytes(bytes[chain_offset..chain_offset + 4].try_into().unwrap()) as usize;
    let counts_offset = chain_offset + 4 + chain * 72;
    for (index, expected) in [1u64, 0, 2, 1].iter().enumerate() {
        assert_eq!(
            u64::from_le_bytes(
                bytes[counts_offset + index * 8..counts_offset + index * 8 + 8]
                    .try_into()
                    .unwrap()
            ),
            *expected
        );
    }
    assert_eq!(
        u32::from_le_bytes(
            bytes[counts_offset + 32..counts_offset + 36]
                .try_into()
                .unwrap()
        ),
        6
    );
    fn stat(metadata: fs::Metadata) -> Vec<u8> {
        use std::os::unix::fs::MetadataExt;
        [
            metadata.dev(),
            metadata.ino(),
            u64::from(metadata.mode()),
            metadata.size(),
            metadata.mtime() as u64,
            metadata.mtime_nsec() as u64,
            metadata.ctime() as u64,
            metadata.ctime_nsec() as u64,
            metadata.nlink(),
        ]
        .into_iter()
        .flat_map(u64::to_le_bytes)
        .collect()
    }
    let mut expected = evidence_record(&[], 3);
    expected.extend(stat(fs::metadata(&root).unwrap()));
    expected.extend_from_slice(&3u32.to_le_bytes());
    for name in [b".avc".as_slice(), b"empty", b"z"] {
        evidence_raw(&mut expected, name);
    }
    expected.extend(evidence_record(&[b".avc"], 4));
    expected.extend(evidence_record(&[b"empty"], 3));
    expected.extend(stat(fs::metadata(root.join("empty")).unwrap()));
    expected.extend_from_slice(&0u32.to_le_bytes());
    expected.extend(evidence_record(&[b"z"], 1));
    expected.extend(stat(fs::metadata(root.join("z")).unwrap()));
    expected.extend_from_slice(&5u64.to_le_bytes());
    expected.extend(evidence_record(&[b".git"], 4));
    expected.extend(evidence_record(&[b"gone"], 5));
    assert_eq!(&bytes[counts_offset + 36..], expected);
    let counts = report(bytes).unwrap();
    assert_eq!(counts.excluded, vec![path(&[b".avc"]), path(&[b".git"])]);
    assert_eq!(counts.removed, vec![path(&[b"gone"])]);
}

#[test]
fn report_refuses_positive_observations_that_contradict_coverage_or_absence() {
    for (removed, records) in [
        (
            1,
            vec![
                evidence_scanned(&[], &[]),
                evidence_scanned(&[b"gone"], &[]),
                evidence_record(&[b"gone"], 5),
            ],
        ),
        (
            0,
            vec![
                evidence_scanned(&[], &[]),
                evidence_scanned(&[b"gone"], &[]),
            ],
        ),
        (
            1,
            vec![
                evidence_scanned(&[], &[]),
                evidence_scanned(&[b"gone", b"deep"], &[]),
                evidence_record(&[b"gone"], 5),
            ],
        ),
        (
            1,
            vec![
                evidence_scanned(&[], &[]),
                evidence_scanned(&[b"gone", b"deep"], &[]),
                evidence_record(&[b"gone", b"deep", b"old"], 5),
            ],
        ),
    ] {
        assert!(report(&evidence_fixture(2, [0, 0, 2, removed], &records)).is_err());
    }
    let drops_exclusion = evidence_fixture(
        2,
        [0, 0, 1, 1],
        &[
            evidence_scanned(&[], &[]),
            evidence_record(&[b"gone"], 5),
            evidence_record(&[b"gone", b".avc"], 4),
        ],
    );
    assert!(report(&drops_exclusion).is_err());
}

#[test]
fn report_charges_base_only_exclusions_to_the_expanded_entry_limit() {
    let names: Vec<_> = (0..6000)
        .map(|n| format!("dir{n:04}").into_bytes())
        .collect();
    let mut records = vec![evidence_scanned(
        &[],
        &names.iter().map(Vec::as_slice).collect::<Vec<_>>(),
    )];
    for name in &names {
        records.push(evidence_scanned(&[name], &[]));
        for administration in [b".avc", b".git"] {
            for bits in 0..8 {
                let variant: Vec<u8> = administration
                    .iter()
                    .enumerate()
                    .map(|(i, b)| {
                        if i > 0 && bits & (1 << (i - 1)) != 0 {
                            b.to_ascii_uppercase()
                        } else {
                            *b
                        }
                    })
                    .collect();
                records.push(evidence_record(&[name, &variant], 4));
            }
        }
    }
    let input = evidence_fixture(2, [0, 0, 6001, 0], &records);
    assert!(input.len() < avc::capture::MAX_WITNESS_BYTES);
    assert!(report(&input).is_err());
}

#[test]
fn report_requires_immediate_scanned_parent_and_actual_raw_name_absence() {
    let absent = evidence_record(&[b"parent", b"gone"], 5);
    let valid = evidence_fixture(
        2,
        [0, 0, 1, 1],
        &[
            evidence_scanned(&[b"parent"], &[b"present"]),
            absent.clone(),
        ],
    );
    assert_eq!(
        report(&valid).unwrap().removed,
        vec![path(&[b"parent", b"gone"])]
    );
    let mut unscanned = evidence_record(&[b"parent"], 0);
    unscanned.extend(evidence_directory_stat());
    for records in [
        vec![absent.clone()],
        vec![unscanned, absent.clone()],
        vec![evidence_scanned(&[], &[b"parent"]), absent.clone()],
        vec![evidence_scanned(&[b"other"], &[]), absent.clone()],
        vec![
            evidence_scanned(&[b"parent"], &[b"gone", b"present"]),
            absent,
        ],
    ] {
        assert!(report(&evidence_fixture(2, [0, 0, 1, 1], &records)).is_err());
    }
    // All truncations include loss of an EOF name-list field or absence path.
    for length in 0..valid.len() {
        assert!(report(&valid[..length]).is_err());
    }
}

#[test]
fn report_refuses_duplicate_absence_administrative_paths_invalid_modes_and_old_versions() {
    for mode in [0, 1, 3, 255] {
        let input = evidence_fixture(
            mode,
            [0, 0, 1, 1],
            &[evidence_scanned(&[], &[]), evidence_record(&[b"gone"], 5)],
        );
        assert!(report(&input).is_err());
    }
    for path in [
        vec![],
        vec![b".AvC".as_slice()],
        vec![b"a".as_slice(), b".gIT"],
    ] {
        let input = evidence_fixture(
            2,
            [0, 0, 1, 1],
            &[evidence_scanned(&[], &[]), evidence_record(&path, 5)],
        );
        assert!(report(&input).is_err());
    }
    let absent = evidence_record(&[b"gone"], 5);
    assert!(
        report(&evidence_fixture(
            2,
            [0, 0, 1, 2],
            &[evidence_scanned(&[], &[]), absent.clone(), absent]
        ))
        .is_err()
    );
    for version in *b"124" {
        let mut input = evidence_fixture(2, [0, 0, 1, 0], &[evidence_scanned(&[], &[])]);
        input[7] = version;
        assert!(report(&input).is_err());
    }
    assert_eq!(CaptureMode::try_from(0).unwrap(), CaptureMode::Leaves);
    assert_eq!(CaptureMode::try_from(1).unwrap(), CaptureMode::Recursive);
    assert_eq!(
        CaptureMode::try_from(2).unwrap(),
        CaptureMode::RecursiveRemovingMissing
    );
    assert!(CaptureMode::try_from(3).is_err());
}

#[test]
fn report_refuses_malformed_raw_name_lists_counts_and_strict_framing() {
    for names in [
        vec![b"z".as_slice(), b"a"],
        vec![b"a".as_slice(), b"a"],
        vec![b"".as_slice()],
        vec![b"a/b".as_slice()],
        vec![b"nul\0".as_slice()],
    ] {
        assert!(
            report(&evidence_fixture(
                2,
                [0, 0, 1, 0],
                &[evidence_scanned(&[], &names)]
            ))
            .is_err()
        );
    }
    let scanned = evidence_scanned(&[], &[]);
    assert!(
        report(&evidence_fixture(
            2,
            [0, 0, 2, 0],
            &[scanned.clone(), scanned.clone()]
        ))
        .is_err()
    );
    let mut huge_count = scanned.clone();
    huge_count[77..81].copy_from_slice(&100_001u32.to_le_bytes());
    assert!(report(&evidence_fixture(2, [0, 0, 1, 0], &[huge_count])).is_err());
    let mut oversized_name = evidence_scanned(&[], &[b"a"]);
    oversized_name[81..85].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(report(&evidence_fixture(2, [0, 0, 1, 0], &[oversized_name])).is_err());
    let records = [scanned, evidence_record(&[b"gone"], 5)];
    for counts in [[1, 0, 1, 1], [0, 1, 1, 1], [0, 0, 2, 1], [0, 0, 1, 0]] {
        assert!(report(&evidence_fixture(2, counts, &records)).is_err());
    }
    let valid = evidence_fixture(2, [0, 0, 1, 1], &records);
    let counts_offset = 8 + 1 + 4 + 1 + 4 + 72;
    for count in [0u32, 1, 3, u32::MAX] {
        let mut bad = valid.clone();
        bad[counts_offset + 32..counts_offset + 36].copy_from_slice(&count.to_le_bytes());
        assert!(report(&bad).is_err());
    }
    let mut trailing = valid;
    trailing.push(0);
    assert!(report(&trailing).is_err());
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
    let chain_offset = 13 + root.as_os_str().as_bytes().len();
    let chain =
        u32::from_le_bytes(bad[chain_offset..chain_offset + 4].try_into().unwrap()) as usize;
    let counts_offset = chain_offset + 4 + chain * 72;
    bad[counts_offset] = 2;
    assert!(report(&bad).is_err());
    let mut bad = observed.witnesses.clone();
    let record_offset = counts_offset + 36;
    bad[record_offset..record_offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(report(&bad).is_err());
    assert!(report(&vec![0; MAX_WITNESS_BYTES + 1]).is_err());
}
