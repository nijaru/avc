#[cfg(unix)]
mod unix {
    use avc::{
        Conflict, Entry, Id, Node, Object,
        engine::{Edit, Engine, Limits},
        materialize::{Error, restore, restore_with_limits},
        repository::Repository,
    };
    use std::{
        fs,
        os::unix::{
            ffi::OsStrExt,
            fs::{PermissionsExt, symlink},
        },
        path::Path,
    };

    fn put(engine: &mut Engine, root: Id, name: &[u8], value: Entry) -> Id {
        engine
            .edit(
                root,
                &[Edit {
                    path: vec![name.to_vec()],
                    expected: Entry::Absent,
                    value,
                }],
            )
            .unwrap()
    }
    fn file(engine: &mut Engine, bytes: &[u8], executable: bool) -> Entry {
        Entry::File {
            blob: engine.blob(bytes.to_vec()).unwrap(),
            executable,
        }
    }
    fn records(engine: &Engine) -> Vec<(Id, Vec<u8>)> {
        engine
            .objects()
            .map(|(id, o)| (id, o.encode().unwrap()))
            .collect()
    }
    fn assert_absent(path: &Path) {
        assert_eq!(
            fs::symlink_metadata(path).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
    }
    fn import_object(engine: &mut Engine, object: Object) -> Id {
        let id = object.id().unwrap();
        let mut records = records(engine);
        records.push((id, object.encode().unwrap()));
        let root = if matches!(object, Object::Conflict(Conflict { region: 0, .. })) {
            let wrapper = Object::Map(Node::Leaf(vec![(
                b"fixture".iter().flat_map(|b| [b >> 4, b & 15]).collect(),
                Entry::Conflict(id),
            )]));
            let root = wrapper.id().unwrap();
            records.push((root, wrapper.encode().unwrap()));
            root
        } else {
            id
        };
        engine.import(records, root, Limits::default()).unwrap();
        id
    }
    fn conflict(a: Entry, b: Entry, base: Entry, region: u8) -> Object {
        let mut positive = vec![(a, 1), (b, 1)];
        positive.sort_by_key(|(e, _)| e.encode().unwrap());
        Object::Conflict(Conflict {
            region,
            positive,
            negative: vec![(base, 1)],
        })
    }

    #[test]
    fn source_store_export_import_restore_preserves_exact_source_and_retention() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().canonicalize().unwrap();
        let mut engine = Engine::new();
        let mut root = engine.empty().unwrap();
        let empty = root;
        let binary = b"\x00\xff\xfe\r\n\x00not text";
        let ordinary = file(&mut engine, binary, false);
        let executable = file(&mut engine, b"#!/bin/sh\necho source\n", true);
        root = put(&mut engine, root, b"binary", ordinary);
        root = put(&mut engine, root, b"run", executable);
        root = put(&mut engine, root, b"empty", Entry::Directory(empty));
        // No admin ownership in a fresh source directory: even a source file
        // named .avc must round trip rather than being hidden or initialized.
        let admin_name = file(&mut engine, b"ordinary source .avc", false);
        root = put(&mut engine, root, b".avc", admin_name);
        let raw_target = b"../../outside/\xff/target";
        let target = engine.blob(raw_target.to_vec()).unwrap();
        root = put(&mut engine, root, b"link", Entry::Symlink(target));
        let nested_file = file(&mut engine, b"nested bytes", false);
        let nested = put(&mut engine, empty, b"child", nested_file);
        root = put(
            &mut engine,
            root,
            "non-ascii-é".as_bytes(),
            Entry::Directory(nested),
        );
        let original = records(&engine);

        let source_project = path.join("source-project");
        fs::create_dir(&source_project).unwrap();
        fs::write(source_project.join("untracked"), b"keep project data").unwrap();
        let mut source = Repository::init(&source_project).unwrap();
        source.commit_source(&engine, root, 0, "source").unwrap();
        let mut export = Vec::new();
        source.export(root, &mut export).unwrap();
        let target_project = path.join("imported-store");
        fs::create_dir(&target_project).unwrap();
        let mut imported = Repository::init(&target_project).unwrap();
        imported.import(export.as_slice(), "import").unwrap();
        let (selection, restored_engine) = imported.load().unwrap();
        assert_eq!(selection.root, root);
        let fresh = path.join("fresh");
        restore(&restored_engine, root, &fresh).unwrap();
        assert_eq!(fs::read(fresh.join("binary")).unwrap(), binary);
        assert_eq!(
            fs::read(fresh.join("run")).unwrap(),
            b"#!/bin/sh\necho source\n"
        );
        assert_eq!(
            fs::metadata(fresh.join("run"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
        assert_eq!(
            fs::metadata(fresh.join("binary"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o644
        );
        assert_eq!(
            fs::read(fresh.join(".avc")).unwrap(),
            b"ordinary source .avc"
        );
        assert_eq!(
            fs::read(fresh.join("non-ascii-é/child")).unwrap(),
            b"nested bytes"
        );
        assert!(fresh.join("empty").is_dir());
        assert_eq!(fs::read_dir(fresh.join("empty")).unwrap().count(), 0);
        assert!(
            fs::symlink_metadata(fresh.join("link"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_link(fresh.join("link"))
                .unwrap()
                .as_os_str()
                .as_bytes(),
            raw_target
        );
        assert_eq!(
            fs::read(source_project.join("untracked")).unwrap(),
            b"keep project data"
        );
        assert_eq!(
            source
                .load_root(root)
                .unwrap()
                .read(root, &[b"binary".to_vec()])
                .unwrap(),
            restored_engine.read(root, &[b"binary".to_vec()]).unwrap()
        );
        assert_eq!(records(&engine), original);
        assert_eq!(imported.status().unwrap(), selection);
    }

    #[test]
    fn every_existing_destination_is_refused_without_changes() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().canonicalize().unwrap();
        let mut engine = Engine::new();
        let root = engine.empty().unwrap();
        fs::create_dir(path.join("empty")).unwrap();
        fs::create_dir(path.join("populated")).unwrap();
        fs::write(path.join("populated/precious"), b"keep").unwrap();
        fs::write(path.join("file"), b"original").unwrap();
        symlink("populated", path.join("link")).unwrap();
        symlink("never-existed", path.join("dangling")).unwrap();
        for name in ["empty", "populated", "file", "link", "dangling"] {
            assert!(
                matches!(restore(&engine, root, &path.join(name)), Err(Error::Io { source, .. })
                if source.kind() == std::io::ErrorKind::AlreadyExists)
            );
        }
        assert_eq!(fs::read_dir(path.join("empty")).unwrap().count(), 0);
        assert_eq!(fs::read(path.join("populated/precious")).unwrap(), b"keep");
        assert_eq!(fs::read(path.join("file")).unwrap(), b"original");
        assert_eq!(
            fs::read_link(path.join("link")).unwrap(),
            Path::new("populated")
        );
        assert_eq!(
            fs::read_link(path.join("dangling")).unwrap(),
            Path::new("never-existed")
        );
        assert_absent(&path.join("never-existed"));
    }

    #[test]
    fn symlink_destination_parent_is_not_followed() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().canonicalize().unwrap();
        fs::create_dir(path.join("outside")).unwrap();
        symlink("outside", path.join("alias")).unwrap();
        let mut engine = Engine::new();
        let root = engine.empty().unwrap();
        assert!(restore(&engine, root, &path.join("alias/fresh")).is_err());
        assert_absent(&path.join("outside/fresh"));
        assert_eq!(fs::read_dir(path.join("outside")).unwrap().count(), 0);
    }

    #[test]
    fn conflicts_at_root_or_nested_path_refuse_before_creation() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().canonicalize().unwrap();
        let mut engine = Engine::new();
        let empty = engine.empty().unwrap();
        let a = file(&mut engine, b"a", false);
        let b = file(&mut engine, b"b", false);
        let base = file(&mut engine, b"base", false);
        let unresolved = import_object(&mut engine, conflict(a.clone(), b.clone(), base, 0));
        let nested = put(
            &mut engine,
            empty,
            b"unresolved",
            Entry::Conflict(unresolved),
        );
        let good = file(&mut engine, b"would otherwise create", false);
        let with_good = put(&mut engine, nested, b"a-first", good);
        let root = put(&mut engine, empty, b"nested", Entry::Directory(with_good));
        assert!(matches!(restore(&engine, root, &path.join("nested")),
            Err(Error::Source(avc::Error::Conflict(id))) if id == unresolved));
        assert_absent(&path.join("nested"));

        let left = put(&mut engine, empty, b"left", a);
        let right = put(&mut engine, empty, b"right", b);
        let unresolved = import_object(
            &mut engine,
            conflict(
                Entry::Directory(left),
                Entry::Directory(right),
                Entry::Directory(empty),
                1,
            ),
        );
        assert!(matches!(restore(&engine, unresolved, &path.join("root")),
            Err(Error::Source(avc::Error::Conflict(id))) if id == unresolved));
        assert_absent(&path.join("root"));
    }

    #[test]
    fn unsupported_symlink_targets_refuse_before_any_output() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().canonicalize().unwrap();
        let mut targets = vec![Vec::new(), b"target\0suffix".to_vec()];
        #[cfg(target_os = "macos")]
        targets.push(vec![b'x'; 1024]);
        #[cfg(target_os = "linux")]
        targets.push(vec![b'x'; 4096]);
        for target in targets {
            let mut engine = Engine::new();
            let empty = engine.empty().unwrap();
            let good = file(&mut engine, b"good", false);
            let root = put(&mut engine, empty, b"a-first", good);
            let id = engine.blob(target).unwrap();
            let nested = put(&mut engine, empty, b"bad-target", Entry::Symlink(id));
            let root = put(&mut engine, root, b"z-nested", Entry::Directory(nested));
            let before = records(&engine);
            assert!(
                matches!(restore(&engine, root, &path.join("fresh")), Err(Error::UnsupportedSymlink { blob, .. }) if blob == id)
            );
            assert_absent(&path.join("fresh"));
            assert_eq!(records(&engine), before);
        }
    }

    #[test]
    fn malformed_names_cannot_enter_authority_or_mutate_destination() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().canonicalize().unwrap();
        for name in [
            b"".as_slice(),
            b".",
            b"..",
            b"escape/child",
            b"nul\0suffix",
            &[b'x'; 256],
        ] {
            let mut engine = Engine::new();
            let empty = engine.empty().unwrap();
            let value = file(&mut engine, b"source", false);
            assert!(
                engine
                    .edit(
                        empty,
                        &[Edit {
                            path: vec![name.to_vec()],
                            expected: Entry::Absent,
                            value: value.clone()
                        }]
                    )
                    .is_err()
            );
            // The codec permits radix fragments, but rooted admission must reject
            // a fragment that recovers to an invalid complete basename.
            let object = Object::Map(Node::Leaf(vec![(
                name.iter().flat_map(|b| [b >> 4, b & 15]).collect(),
                value,
            )]));
            let Ok(payload) = object.encode() else {
                continue;
            }; // >510 nibbles is already codec-invalid.
            let id = object.id().unwrap();
            let mut input = records(&engine);
            input.push((id, payload));
            assert!(matches!(
                engine.import(input, id, Limits::default()),
                Err(avc::Error::Invalid(_))
            ));
            assert!(matches!(
                restore(&engine, id, &path.join("fresh")),
                Err(Error::Source(avc::Error::NeedObjects(_)))
            ));
            assert_absent(&path.join("fresh"));
        }
    }

    #[test]
    fn missing_source_and_non_source_root_refuse_before_creation() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().canonicalize().unwrap();
        let mut engine = Engine::new();
        let missing = Id::hash(avc::MAP_LEAF_KIND, b"missing").unwrap();
        assert!(
            matches!(restore(&engine, missing, &path.join("fresh")), Err(Error::Source(avc::Error::NeedObjects(id))) if id == missing)
        );
        let blob = engine.blob(b"not a root".to_vec()).unwrap();
        assert!(matches!(
            restore(&engine, blob, &path.join("fresh")),
            Err(Error::Source(avc::Error::Invalid(_)))
        ));
        assert_absent(&path.join("fresh"));
    }

    #[test]
    fn expanded_shared_subtrees_are_budgeted_before_creation() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().canonicalize().unwrap();
        let mut engine = Engine::new();
        let empty = engine.empty().unwrap();
        let value = file(&mut engine, &[0xab; 2048], false);
        let child = put(&mut engine, empty, b"file", value);
        let mut root = empty;
        for name in [b"a", b"b", b"c", b"d", b"e", b"f", b"g", b"h"] {
            root = put(&mut engine, root, name, Entry::Directory(child));
        }
        for limits in [
            Limits {
                bytes: 10_000,
                ..Limits::default()
            },
            Limits {
                objects: 4,
                ..Limits::default()
            },
            Limits {
                bindings: 12,
                ..Limits::default()
            },
        ] {
            // Verification sees the shared source once; materialization has
            // strictly more entries/map visits/bytes and must bound expansion.
            engine.verify(root, limits).unwrap();
            assert!(matches!(
                restore_with_limits(&engine, root, &path.join("fresh"), limits),
                Err(Error::Source(avc::Error::Limit(_)))
            ));
            assert_absent(&path.join("fresh"));
        }
    }

    #[test]
    fn wide_radix_with_terminal_and_deep_directories_round_trips() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().canonicalize().unwrap();
        let mut engine = Engine::new();
        let empty = engine.empty().unwrap();
        let value = file(&mut engine, b"bytes", false);
        let mut wide = put(&mut engine, empty, b"a", value.clone());
        for i in 0..80 {
            wide = put(
                &mut engine,
                wide,
                format!("a{i:03}").as_bytes(),
                value.clone(),
            );
        }
        let mut root = wide;
        for _ in 0..32 {
            root = put(&mut engine, empty, b"d", Entry::Directory(root));
        }
        let fresh = path.join("fresh");
        restore(&engine, root, &fresh).unwrap();
        let mut leaf = fresh.clone();
        for _ in 0..32 {
            leaf.push("d");
        }
        assert_eq!(fs::read(leaf.join("a")).unwrap(), b"bytes");
        for i in 0..80 {
            assert_eq!(fs::read(leaf.join(format!("a{i:03}"))).unwrap(), b"bytes");
        }
        assert_eq!(fs::read_dir(&leaf).unwrap().count(), 81);
        assert_absent(&fresh.join(".avc"));
    }

    #[test]
    fn raw_names_are_preserved_or_preflight_refused_on_macos() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().canonicalize().unwrap();
        let mut engine = Engine::new();
        let empty = engine.empty().unwrap();
        let value = file(&mut engine, b"raw", false);
        let root = put(&mut engine, empty, b"raw-\xff", value);
        let fresh = path.join("fresh");
        #[cfg(target_os = "macos")]
        {
            assert!(matches!(
                restore(&engine, root, &fresh),
                Err(Error::UnsupportedName { .. })
            ));
            assert_absent(&fresh);
        }
        #[cfg(not(target_os = "macos"))]
        {
            restore(&engine, root, &fresh).unwrap();
            assert_eq!(
                fs::read(fresh.join(std::ffi::OsStr::from_bytes(b"raw-\xff"))).unwrap(),
                b"raw"
            );
        }
    }

    #[test]
    fn filesystem_aliases_never_overwrite_and_partial_output_is_left_intact() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().canonicalize().unwrap();
        for (first, alias) in [("A", "a"), ("e\u{301}", "é")] {
            // Query this actual filesystem's equivalence behavior independently.
            let probe = path.join("probe");
            fs::create_dir(&probe).unwrap();
            fs::write(probe.join(first), b"probe").unwrap();
            let aliases = probe.join(alias).exists();
            fs::remove_dir_all(&probe).unwrap();
            let mut engine = Engine::new();
            let empty = engine.empty().unwrap();
            let a = file(&mut engine, b"first", false);
            let b = file(&mut engine, b"second", false);
            let root = put(&mut engine, empty, first.as_bytes(), a);
            let root = put(&mut engine, root, alias.as_bytes(), b);
            let fresh = path.join("fresh");
            if aliases {
                assert!(
                    matches!(restore(&engine, root, &fresh), Err(Error::Io { source, .. }) if source.kind() == std::io::ErrorKind::AlreadyExists)
                );
                assert_eq!(fs::read_dir(&fresh).unwrap().count(), 1);
                assert_eq!(fs::read(fresh.join(first)).unwrap(), b"first");
                // A retry is not a resume protocol; it must refuse this partial root.
                assert!(restore(&engine, root, &fresh).is_err());
            } else {
                restore(&engine, root, &fresh).unwrap();
                assert_eq!(fs::read(fresh.join(first)).unwrap(), b"first");
                assert_eq!(fs::read(fresh.join(alias)).unwrap(), b"second");
            }
            fs::remove_dir_all(fresh).unwrap();
        }
    }
}

#[cfg(not(unix))]
#[test]
fn unsupported_platform_refuses_without_writes() {
    use avc::{
        engine::Engine,
        materialize::{Error, restore},
    };
    let temp = tempfile::tempdir().unwrap();
    let mut engine = Engine::new();
    let root = engine.empty().unwrap();
    let fresh = temp.path().join("fresh");
    assert!(matches!(
        restore(&engine, root, &fresh),
        Err(Error::UnsupportedPlatform)
    ));
    assert!(!fresh.exists());
}
