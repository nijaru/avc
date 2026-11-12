#![cfg(any(target_os = "linux", target_os = "macos"))]

use avc::{Entry, Id, Object, capture::observe, engine::Edit, engine::Engine};
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
    match observe(&mut engine, base, root, paths) {
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
    let observed = observe(&mut engine, base, &root, &selected).unwrap();
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
    assert!(observed.witnesses.starts_with(b"AVCOBS01"));
    assert!(observed.witnesses.len() <= 256 * 1024);
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
    let observed = observe(&mut engine, base, &root, &[path(&[name])]).unwrap();
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
    )
    .unwrap();
    let mut input = observation.witnesses.as_slice();
    assert_eq!(&take::<8>(&mut input), b"AVCOBS01");
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
    assert_eq!(count(&mut input), 2);
    for (name, tag) in [(b"file".as_slice(), 1), (b"link".as_slice(), 2)] {
        assert_eq!(count(&mut input), 1);
        assert_eq!(raw(&mut input), name);
        assert_eq!(count(&mut input), 0);
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
    let observed = observe(&mut engine, base, &root, &[path(&[b"nested", b"selected"])]).unwrap();
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
        let error = observe(&mut engine, base, &root, &[path(&[b"first"]), selected]);
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
    assert!(observe(&mut engine, base, &root, &[path(&[b"huge"])]).is_err());
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
    assert!(observe(&mut engine, base, &root, &[path(&[b"directory"])]).is_err());
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
fn witness_metadata_bound_explicitly_refuses_deep_repeated_parents() {
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
    assert!(refuses(&root, &selected).contains("metadata exceeds 256 KiB"));
}

#[test]
fn capture_accepts_the_exact_native_blob_payload_limit_and_refuses_one_more_byte() {
    let (_temp, root) = temp();
    let file = fs::File::create(root.join("boundary")).unwrap();
    file.set_len(16 * 1024 * 1024).unwrap();
    let mut engine = Engine::new();
    let base = engine.empty().unwrap();
    let selected = path(&[b"boundary"]);
    let observed = observe(&mut engine, base, &root, std::slice::from_ref(&selected)).unwrap();
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
    assert!(observe(&mut engine, base, &root, &[path(&[b"selected"])]).is_err());
}
