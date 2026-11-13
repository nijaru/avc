use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use avc::engine::{Edit, Engine};
use avc::git::{GitCommit, GitOid, GitPlan, GitStore, GitTreeHasher};
use avc::{Entry, Object, Revision};
use tempfile::TempDir;

// Fixtures are all locally generated, with bounded stdin/stdout and no refs.
fn git(dir: &Path, args: &[&str], input: &[u8]) -> Vec<u8> {
    let mut child = Command::new("git")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .arg("--git-dir")
        .arg(dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let mut output = Vec::new();
    child
        .stdout
        .take()
        .unwrap()
        .take(17 * 1024 * 1024)
        .read_to_end(&mut output)
        .unwrap();
    assert!(child.wait().unwrap().success(), "{args:?}");
    output
}
fn init() -> (TempDir, GitStore) {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "--bare", "--quiet"], &[]);
    let store = GitStore::open(dir.path()).unwrap();
    (dir, store)
}
fn put(dir: &Path, kind: &str, raw: &[u8]) -> GitOid {
    let output = git(
        dir,
        &["hash-object", "--literally", "-w", "-t", kind, "--stdin"],
        raw,
    );
    std::str::from_utf8(&output)
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}
fn tree_entry(raw: &mut Vec<u8>, mode: &[u8], name: &[u8], oid: GitOid) {
    raw.extend_from_slice(mode);
    raw.push(b' ');
    raw.extend_from_slice(name);
    raw.push(0);
    raw.extend_from_slice(&oid.to_bytes()[1..]);
}
fn raw_commit(tree: GitOid) -> Vec<u8> {
    format!("tree {}\nauthor Original Name <author@example.test> 99 -0530\ncommitter Other Person <committer@example.test> 123 +1245\nextra opaque\n continuation preserved\n\nexact message\nwithout final newline", bare(tree)).into_bytes()
}
fn bare(oid: GitOid) -> String {
    oid.to_string()[5..].to_owned()
}
fn revision(root: avc::Id) -> Revision {
    Revision {
        root,
        parents: vec![],
        change: [9; 16],
        author: "Native Name <native@example.test>".into(),
        message: "message without newline".into(),
        time_ns: 1_234_567_890,
        timezone_minutes: -1439,
    }
}
fn object_files(dir: &Path) -> usize {
    std::fs::read_dir(dir.join("objects"))
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            if path.file_name().unwrap().len() == 2 {
                std::fs::read_dir(path).unwrap().count()
            } else {
                0
            }
        })
        .sum()
}

#[test]
fn oid_persistence_is_algorithm_tagged() {
    let oid: GitOid = "0123456789ABCDEF0123456789abcdef01234567".parse().unwrap();
    assert_eq!(
        oid.to_string(),
        "sha1:0123456789abcdef0123456789abcdef01234567"
    );
    assert_eq!(GitOid::from_bytes(&oid.to_bytes()).unwrap(), oid);
    assert!(GitOid::from_bytes(&oid.to_bytes()[1..]).is_err());
    let mut bytes = oid.to_bytes();
    bytes[0] = 2;
    assert!(GitOid::from_bytes(&bytes).is_err());
    assert!(
        "sha256:0123456789abcdef0123456789abcdef01234567"
            .parse::<GitOid>()
            .is_err()
    );
}

#[test]
fn exact_git_source_roundtrip_and_native_child_metadata() {
    let (dir, store) = init();
    let blob = put(dir.path(), "blob", b"\0raw\ncontent\xff");
    let link = put(dir.path(), "blob", b"../target\xff");
    let pointer = put(
        dir.path(),
        "blob",
        b"version https://git-lfs.github.com/spec/v1\noid sha256:123\nsize 999\n",
    );
    let mut nested = Vec::new();
    tree_entry(&mut nested, b"100755", b"exec", blob);
    let nested = put(dir.path(), "tree", &nested);
    let mut raw = Vec::new();
    tree_entry(&mut raw, b"100644", b"a.c", blob);
    tree_entry(&mut raw, b"40000", b"a", nested); // Git's directory terminator sorts '/' after '.'
    tree_entry(&mut raw, b"120000", b"link", link);
    tree_entry(&mut raw, b"100644", b"pointer", pointer);
    tree_entry(&mut raw, b"100644", b"raw-\xff", blob);
    let tree = put(dir.path(), "tree", &raw);
    let commit_raw = raw_commit(tree);
    let parent = put(dir.path(), "commit", &commit_raw);
    let commit = store.read_commit(parent).unwrap();
    assert_eq!(commit.raw, commit_raw);
    assert_eq!(commit.author, "Original Name <author@example.test>");
    assert_eq!(commit.time_ns, 123_000_000_000);
    assert_eq!(commit.timezone_minutes, 765);
    assert_eq!(commit.message, "exact message\nwithout final newline");
    let mut engine = Engine::new();
    let root = store.import_source(tree, &mut engine).unwrap();
    assert!(matches!(
        engine
            .read(root, &[b"a".to_vec(), b"exec".to_vec()])
            .unwrap(),
        Entry::File {
            executable: true,
            ..
        }
    ));
    assert!(matches!(
        engine.read(root, &[b"link".to_vec()]).unwrap(),
        Entry::Symlink(_)
    ));
    // The Engine may contain history: only the selected source is exported.
    let anchor = Object::Revision(revision(root));
    let anchor_id = anchor.id().unwrap();
    engine
        .import(
            [(anchor_id, anchor.encode().unwrap())].into_iter().chain(
                engine
                    .objects()
                    .map(|(id, o)| (id, o.encode().unwrap()))
                    .collect::<Vec<_>>(),
            ),
            anchor_id,
            Default::default(),
        )
        .unwrap();
    let mut plan = GitPlan::new();
    assert_eq!(plan.tree(&store, &engine, root).unwrap(), tree);
    let mut child = revision(root);
    child.parents.push(anchor_id);
    let child_id = Object::Revision(child.clone()).id().unwrap();
    let exported = plan.commit(&store, child_id, &child, &[parent]).unwrap();
    let before = object_files(dir.path());
    assert_eq!(
        git(dir.path(), &["cat-file", "-e", &bare(parent)], &[]),
        b""
    );
    assert_eq!(object_files(dir.path()), before);
    plan.write(&store).unwrap();
    let actual = git(dir.path(), &["cat-file", "commit", &bare(exported)], &[]);
    let expected = format!(
        "tree {}\nparent {}\nauthor Native Name <native@example.test> 1 -2359\ncommitter Native Name <native@example.test> 1 -2359\navc-revision {child_id}\navc-time-ns 1234567890\n\nmessage without newline",
        bare(tree),
        bare(parent)
    );
    assert_eq!(actual, expected.as_bytes());
    assert_eq!(
        git(dir.path(), &["cat-file", "tree", &bare(tree)], &[]),
        raw
    );
    let imported = store.read_commit(exported).unwrap();
    assert_eq!(imported.time_ns, 1_000_000_000); // audit header is not identity input
    assert_eq!(imported.parents, [parent]);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("HEAD"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    assert!(git(dir.path(), &["for-each-ref"], &[]).is_empty());
    git(dir.path(), &["fsck", "--strict", "--no-reflogs"], &[]);
}

#[test]
fn invalid_source_is_atomic_and_modes_paths_and_order_are_strict() {
    let (dir, store) = init();
    let blob = put(dir.path(), "blob", b"data");
    for (mode, name) in [
        (b"160000".as_slice(), b"submodule".as_slice()),
        (b"100644", b".."),
        (b"100644", b"a/b"),
        (b"100664", b"bad-mode"),
    ] {
        let mut raw = Vec::new();
        tree_entry(&mut raw, mode, name, blob);
        let tree = put(dir.path(), "tree", &raw);
        let mut engine = Engine::new();
        let existing = engine.empty().unwrap();
        assert!(store.import_source(tree, &mut engine).is_err());
        assert_eq!(engine.object_count(), 1);
        assert_eq!(engine.empty().unwrap(), existing);
    }
    for names in [[b"z".as_slice(), b"a".as_slice()], [b"a", b"a"]] {
        let mut raw = Vec::new();
        for name in names {
            tree_entry(&mut raw, b"100644", name, blob);
        }
        let tree = put(dir.path(), "tree", &raw);
        assert!(store.import_source(tree, &mut Engine::new()).is_err());
    }
    let mut raw = Vec::new();
    tree_entry(&mut raw, b"40000", b"wrong-type", blob);
    let tree = put(dir.path(), "tree", &raw);
    assert!(store.import_source(tree, &mut Engine::new()).is_err());
}

#[test]
fn corrupt_existing_oid_is_not_trusted_on_read_or_write() {
    let (dir, store) = init();
    let good = put(dir.path(), "blob", b"good");
    let bad = put(dir.path(), "blob", b"evil");
    let object_path = |oid| {
        let hex = bare(oid);
        dir.path().join("objects").join(&hex[..2]).join(&hex[2..])
    };
    std::fs::remove_file(object_path(good)).unwrap();
    std::fs::copy(object_path(bad), object_path(good)).unwrap();
    let mut tree_raw = Vec::new();
    tree_entry(&mut tree_raw, b"100644", b"file", good);
    let tree = put(dir.path(), "tree", &tree_raw);
    assert!(store.import_source(tree, &mut Engine::new()).is_err());
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let blob = engine.blob(b"good".to_vec()).unwrap();
    let root = engine
        .edit(
            empty,
            &[Edit {
                path: vec![b"file".to_vec()],
                expected: Entry::Absent,
                value: Entry::File {
                    blob,
                    executable: false,
                },
            }],
        )
        .unwrap();
    let mut plan = GitPlan::new();
    plan.tree(&store, &engine, root).unwrap();
    assert!(plan.write(&store).is_err());
}

#[test]
fn corrupt_late_target_object_prevents_all_early_writes() {
    let (source_dir, source) = init();
    let blob = put(source_dir.path(), "blob", b"planned blob");
    let mut raw = Vec::new();
    tree_entry(&mut raw, b"100644", b"file", blob);
    let tree = put(source_dir.path(), "tree", &raw);
    let mut engine = Engine::new();
    let root = source.import_source(tree, &mut engine).unwrap();
    let (target_dir, target) = init();
    let evil = put(target_dir.path(), "blob", b"corrupt target slot");
    let object_path = |oid| {
        let hex = bare(oid);
        target_dir
            .path()
            .join("objects")
            .join(&hex[..2])
            .join(&hex[2..])
    };
    // GitPlan orders by OID: the other, lower OID would be written first.
    let late = tree.max(blob);
    std::fs::create_dir_all(object_path(late).parent().unwrap()).unwrap();
    std::fs::copy(object_path(evil), object_path(late)).unwrap();
    let before = object_files(target_dir.path());
    let mut plan = GitPlan::new();
    plan.tree(&target, &engine, root).unwrap();
    assert!(plan.write(&target).is_err());
    assert_eq!(object_files(target_dir.path()), before);
}

#[test]
fn malformed_loose_objects_are_errors_not_missing_and_prevent_all_writes() {
    let (source_dir, source) = init();
    let blob_raw = b"planned blob";
    let blob = put(source_dir.path(), "blob", blob_raw);
    let mut tree_raw = Vec::new();
    tree_entry(&mut tree_raw, b"100644", b"file", blob);
    let tree = put(source_dir.path(), "tree", &tree_raw);
    let mut engine = Engine::new();
    let root = source.import_source(tree, &mut engine).unwrap();
    let rev = revision(root);
    let id = Object::Revision(rev.clone()).id().unwrap();
    let mut source_plan = GitPlan::new();
    source_plan.tree(&source, &engine, root).unwrap();
    let commit = source_plan.commit(&source, id, &rev, &[]).unwrap();
    source_plan.write(&source).unwrap();
    let commit_raw = source.read_commit(commit).unwrap().raw;
    for (kind, oid, raw) in [
        ("blob", blob, blob_raw.as_slice()),
        ("tree", tree, tree_raw.as_slice()),
        ("commit", commit, commit_raw.as_slice()),
    ] {
        let (target_dir, target) = init();
        assert_eq!(put(target_dir.path(), kind, raw), oid);
        let hex = bare(oid);
        let slot = target_dir
            .path()
            .join("objects")
            .join(&hex[..2])
            .join(&hex[2..]);
        // Git's loose files are read-only. Replace rather than overwrite them.
        std::fs::remove_file(&slot).unwrap();
        std::fs::write(&slot, b"not zlib").unwrap();
        let mut plan = GitPlan::new();
        plan.tree(&target, &engine, root).unwrap();
        assert_eq!(plan.commit(&target, id, &rev, &[]).unwrap(), commit);
        // No empty tree is planned: its global fsck must not mask this bug.
        let before = object_files(target_dir.path());
        assert!(plan.write(&target).is_err(), "corrupt {kind} was accepted");
        assert_eq!(
            object_files(target_dir.path()),
            before,
            "early writes before corrupt {kind} refusal"
        );
        assert_eq!(std::fs::read(&slot).unwrap(), b"not zlib");
        if kind == "commit" {
            assert!(target.read_commit_optional(oid).is_err());
        }
    }
}

#[test]
fn raw_objects_ignore_replacements_filters_and_promisor_helpers() {
    let (dir, store) = init();
    let original = put(dir.path(), "blob", b"original");
    let replacement = put(dir.path(), "blob", b"replacement");
    git(
        dir.path(),
        &["replace", &bare(original), &bare(replacement)],
        &[],
    );
    let marker = dir.path().join("executed");
    let evil = format!("!touch {}", marker.display());
    git(dir.path(), &["config", "filter.evil.clean", &evil], &[]);
    git(
        dir.path(),
        &["config", "remote.origin.promisor", "true"],
        &[],
    );
    git(
        dir.path(),
        &[
            "config",
            "remote.origin.url",
            &format!("ext::sh -c 'touch {}'", marker.display()),
        ],
        &[],
    );
    git(dir.path(), &["config", "protocol.ext.allow", "always"], &[]);
    let mut raw = Vec::new();
    tree_entry(&mut raw, b"100644", b"file", original);
    let tree = put(dir.path(), "tree", &raw);
    let mut engine = Engine::new();
    let root = store.import_source(tree, &mut engine).unwrap();
    let Entry::File { blob, .. } = engine.read(root, &[b"file".to_vec()]).unwrap() else {
        panic!()
    };
    assert_eq!(
        engine.object(blob).unwrap(),
        &Object::Blob(b"original".to_vec())
    );
    let missing: GitOid = "1111111111111111111111111111111111111111".parse().unwrap();
    assert!(store.read_commit(missing).is_err());
    assert!(!marker.exists());
}

#[test]
fn malformed_commit_metadata_is_refused_and_extra_headers_retained() {
    let (dir, store) = init();
    let tree = put(dir.path(), "tree", b"");
    let valid = raw_commit(tree);
    for raw in [
        valid.replace_bytes(
            b"author Original",
            b"tree 1111111111111111111111111111111111111111\nauthor Original",
        ),
        valid.replace_bytes(b"123 +1245", b"123 +1260"),
        valid.replace_bytes(b"123 +1245", b"9223372036854775807 +0000"),
        valid.replace_bytes(
            b"author Original Name <author@example.test>",
            b"author MissingEmail",
        ),
        valid.replace_bytes(b"exact message", b"invalid \xff"),
    ] {
        let oid = put(dir.path(), "commit", &raw);
        assert!(store.read_commit(oid).is_err());
    }
    let mut raw = format!("tree {}\n", bare(tree));
    for i in 0..17 {
        raw.push_str(&format!("parent {i:040x}\n"));
    }
    raw.push_str("author A <a> 1 +0000\ncommitter C <c> 1 +0000\n\nm");
    let oid = put(dir.path(), "commit", raw.as_bytes());
    assert!(store.read_commit(oid).is_err());
}
trait ReplaceBytes {
    fn replace_bytes(&self, from: &[u8], to: &[u8]) -> Vec<u8>;
}
impl ReplaceBytes for Vec<u8> {
    fn replace_bytes(&self, from: &[u8], to: &[u8]) -> Vec<u8> {
        let pos = self.windows(from.len()).position(|b| b == from).unwrap();
        [&self[..pos], to, &self[pos + from.len()..]].concat()
    }
}

#[test]
fn all_preflight_errors_precede_writes_and_poison_plan() {
    let (dir, store) = init();
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let blob = engine.blob(vec![1]).unwrap();
    let root = engine
        .edit(
            empty,
            &[Edit {
                path: vec![b"file".to_vec()],
                expected: Entry::Absent,
                value: Entry::File {
                    blob,
                    executable: false,
                },
            }],
        )
        .unwrap();
    let mut plan = GitPlan::new();
    plan.tree(&store, &engine, root).unwrap();
    let mut r = revision(root);
    r.author = "not a Git identity".into();
    let id = Object::Revision(r.clone()).id().unwrap();
    assert!(plan.commit(&store, id, &r, &[]).is_err());
    assert!(plan.write(&store).is_err());
    assert_eq!(object_files(dir.path()), 0);
    assert_eq!(
        GitPlan::new()
            .tree(&store, &engine, empty)
            .unwrap()
            .to_string(),
        "sha1:4b825dc642cb6eb9a060e54bf8d69288fbee4904"
    );
    let root = engine
        .edit(
            root,
            &[Edit {
                path: vec![b"empty-directory".to_vec()],
                expected: Entry::Absent,
                value: Entry::Directory(empty),
            }],
        )
        .unwrap();
    assert!(GitPlan::new().tree(&store, &engine, root).is_err());
}

#[test]
fn root_and_path_conflicts_are_never_projected_to_git() {
    let (dir, store) = init();
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let a = engine.blob(b"a".to_vec()).unwrap();
    let b = engine.blob(b"b".to_vec()).unwrap();
    let file = |blob| Entry::File {
        blob,
        executable: false,
    };
    let root_a = engine
        .edit(
            empty,
            &[Edit {
                path: vec![b"file".to_vec()],
                expected: Entry::Absent,
                value: file(a),
            }],
        )
        .unwrap();
    let root_b = engine
        .edit(
            empty,
            &[Edit {
                path: vec![b"file".to_vec()],
                expected: Entry::Absent,
                value: file(b),
            }],
        )
        .unwrap();
    for region in [0, 1] {
        let mut positive = if region == 0 {
            vec![(Entry::Absent, 1), (file(a), 1)]
        } else {
            vec![(Entry::Directory(root_a), 1), (Entry::Directory(root_b), 1)]
        };
        positive.sort_by_key(|(entry, _)| entry.encode().unwrap());
        let negative = if region == 0 {
            vec![(file(b), 1)]
        } else {
            vec![(Entry::Directory(empty), 1)]
        };
        let conflict = Object::Conflict(avc::Conflict {
            region,
            positive,
            negative,
        });
        let conflict_id = conflict.id().unwrap();
        let mut records: Vec<_> = engine
            .objects()
            .map(|(id, o)| (id, o.encode().unwrap()))
            .collect();
        records.push((conflict_id, conflict.encode().unwrap()));
        let root = if region == 1 {
            conflict_id
        } else {
            let carrier = Object::Map(avc::Node::Leaf(vec![(
                vec![6, 3],
                Entry::Conflict(conflict_id),
            )]));
            let id = carrier.id().unwrap();
            records.push((id, carrier.encode().unwrap()));
            id
        };
        engine.import(records, root, Default::default()).unwrap();
        let mut plan = GitPlan::new();
        assert!(plan.tree(&store, &engine, root).is_err());
        assert!(plan.write(&store).is_err());
    }
    assert_eq!(object_files(dir.path()), 0);
}

#[test]
fn shared_tree_expansion_and_payloads_are_bounded() {
    let (dir, store) = init();
    let blob = put(dir.path(), "blob", b"tiny");
    let mut raw = Vec::new();
    tree_entry(&mut raw, b"100644", b"leaf", blob);
    let mut tree = put(dir.path(), "tree", &raw);
    // Only 18 unique trees, but an exponentially expanded logical source.
    for _ in 0..17 {
        let mut raw = Vec::new();
        tree_entry(&mut raw, b"40000", b"a", tree);
        tree_entry(&mut raw, b"40000", b"b", tree);
        tree = put(dir.path(), "tree", &raw);
    }
    let mut engine = Engine::new();
    assert!(store.import_source(tree, &mut engine).is_err());
    assert_eq!(engine.object_count(), 0);
    let large = put(dir.path(), "blob", &vec![0; 16 * 1024 * 1024 + 1]);
    let mut raw = Vec::new();
    tree_entry(&mut raw, b"100644", b"large", large);
    let tree = put(dir.path(), "tree", &raw);
    assert!(store.import_source(tree, &mut engine).is_err());
    assert_eq!(engine.object_count(), 0);
}

#[test]
fn bulk_radix_import_and_multi_revision_plan_are_canonical_and_deduplicated() {
    let (dir, store) = init();
    let blob = put(dir.path(), "blob", b"shared");
    let mut raw = Vec::new();
    let mut expected = Engine::new();
    let empty = expected.empty().unwrap();
    let native_blob = expected.blob(b"shared".to_vec()).unwrap();
    let mut edits = Vec::new();
    for i in 0..70 {
        let name = format!("prefix-{i:04}").into_bytes();
        tree_entry(&mut raw, b"100644", &name, blob);
        edits.push(Edit {
            path: vec![name],
            expected: Entry::Absent,
            value: Entry::File {
                blob: native_blob,
                executable: false,
            },
        });
    }
    let expected_root = expected.edit(empty, &edits).unwrap();
    let tree = put(dir.path(), "tree", &raw);
    let mut engine = Engine::new();
    let root = store.import_source(tree, &mut engine).unwrap();
    assert_eq!(root, expected_root);
    let (target_dir, target) = init();
    let mut plan = GitPlan::new();
    assert_eq!(plan.tree(&target, &engine, root).unwrap(), tree);
    let mut first = revision(root);
    first.time_ns = 1_234_567_890;
    let first_id = Object::Revision(first.clone()).id().unwrap();
    let first_oid = plan.commit(&target, first_id, &first, &[]).unwrap();
    let mut second = first.clone();
    second.parents.push(first_id);
    second.message = "second".into();
    let second_id = Object::Revision(second.clone()).id().unwrap();
    assert_eq!(plan.tree(&target, &engine, root).unwrap(), tree);
    let second_oid = plan
        .commit(&target, second_id, &second, &[first_oid])
        .unwrap();
    assert_eq!(object_files(target_dir.path()), 0);
    plan.write(&target).unwrap();
    assert_eq!(object_files(target_dir.path()), 4); // one shared blob/tree, two commits
    assert_eq!(target.read_commit(second_oid).unwrap().parents, [first_oid]);
    let mut merge = second.clone();
    merge.parents = vec![second_id, first_id];
    merge.message = "merge".into();
    let merge_id = Object::Revision(merge.clone()).id().unwrap();
    let merge_oid = plan
        .commit(&target, merge_id, &merge, &[second_oid, first_oid])
        .unwrap();
    plan.write(&target).unwrap();
    assert_eq!(
        target.read_commit(merge_oid).unwrap().parents,
        [second_oid, first_oid]
    );
    assert_eq!(
        target.read_commit(first_oid).unwrap().time_ns,
        1_000_000_000
    );
    let mut reimport = Engine::new();
    assert_eq!(target.import_source(tree, &mut reimport).unwrap(), root);
    plan.write(&target).unwrap();
    assert_eq!(object_files(target_dir.path()), 5);
    git(
        target_dir.path(),
        &["fsck", "--no-reflogs", "--strict"],
        &[],
    );
}

#[test]
fn empty_git_tree_import_and_actual_depth_path_bounds() {
    let (dir, store) = init();
    let empty = put(dir.path(), "tree", b"");
    let mut engine = Engine::new();
    let root = store.import_source(empty, &mut engine).unwrap();
    assert_eq!(root, engine.empty().unwrap());
    let blob = put(dir.path(), "blob", b"leaf");
    let mut raw = Vec::new();
    tree_entry(&mut raw, b"100644", b"x", blob);
    let mut tree = put(dir.path(), "tree", &raw);
    for _ in 0..255 {
        let mut raw = Vec::new();
        tree_entry(&mut raw, b"40000", b"x", tree);
        tree = put(dir.path(), "tree", &raw);
    }
    let root = store.import_source(tree, &mut engine).unwrap();
    assert_eq!(GitPlan::new().tree(&store, &engine, root).unwrap(), tree);
    assert_eq!(GitTreeHasher::new().tree(&engine, root).unwrap(), tree);
    let mut raw = Vec::new();
    tree_entry(&mut raw, b"40000", b"x", tree);
    let too_deep = put(dir.path(), "tree", &raw);
    assert!(store.import_source(too_deep, &mut Engine::new()).is_err());
    let mut tree = empty;
    for _ in 0..17 {
        let mut raw = Vec::new();
        tree_entry(&mut raw, b"40000", &[b'x'; 255], tree);
        tree = put(dir.path(), "tree", &raw);
    }
    assert!(store.import_source(tree, &mut Engine::new()).is_err());
}

#[test]
fn deep_git_radix_import_on_two_megabyte_stack() {
    const CASE: &str = "deep_git_radix_import_on_two_megabyte_stack";
    if std::env::var("AVC_GIT_CODEC_STACK_CHILD").as_deref() != Ok(CASE) {
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", CASE, "--nocapture"])
            .env("AVC_GIT_CODEC_STACK_CHILD", CASE)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .status()
            .unwrap();
        assert!(status.success(), "deep Git radix child: {status}");
        return;
    }
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let (dir, store) = init();
            let blob = put(dir.path(), "blob", b"shared");
            let base = vec![b'a'; 255];
            let mut names = vec![base.clone()];
            for i in 0..510 {
                let mut name = base.clone();
                name[i / 2] = if i % 2 == 0 { b'q' } else { b'b' };
                names.push(name);
            }
            names.sort();
            let mut raw = Vec::new();
            for name in &names {
                tree_entry(&mut raw, b"100644", name, blob);
            }
            let tree = put(dir.path(), "tree", &raw);
            let mut engine = Engine::new();
            let root = store.import_source(tree, &mut engine).unwrap();
            let mut expected = Engine::new();
            let empty = expected.empty().unwrap();
            let native_blob = expected.blob(b"shared".to_vec()).unwrap();
            let file = Entry::File {
                blob: native_blob,
                executable: false,
            };
            let edits: Vec<_> = names
                .iter()
                .map(|name| Edit {
                    path: vec![name.clone()],
                    expected: Entry::Absent,
                    value: file.clone(),
                })
                .collect();
            assert_eq!(root, expected.edit(empty, &edits).unwrap());
            assert_eq!(
                engine.list(root, &[]).unwrap(),
                names
                    .into_iter()
                    .map(|name| (name, file.clone()))
                    .collect::<Vec<_>>()
            );
            engine.verify(root, Default::default()).unwrap();
            assert_eq!(GitPlan::new().tree(&store, &engine, root).unwrap(), tree);
            let mut pending = vec![root];
            let mut seen = std::collections::BTreeSet::new();
            while let Some(id) = pending.pop() {
                if !seen.insert(id) {
                    continue;
                }
                match engine.object(id).unwrap() {
                    Object::Blob(_) => {}
                    Object::Map(avc::Node::Leaf(entries)) => {
                        for (_, entry) in entries {
                            let Entry::File { blob, .. } = entry else {
                                panic!("fixture is files only")
                            };
                            pending.push(*blob);
                        }
                    }
                    Object::Map(avc::Node::Branch {
                        terminal, children, ..
                    }) => {
                        if let Some(Entry::File { blob, .. }) = terminal {
                            pending.push(*blob);
                        }
                        pending.extend(children.values());
                    }
                    _ => panic!("unexpected fixture object"),
                }
            }
            assert_eq!(
                seen.len(),
                engine.object_count(),
                "bulk import must not retain orphan intermediate maps"
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn negative_dates_and_nul_messages_are_refused_before_writes() {
    let (dir, store) = init();
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let blob = engine.blob(b"file".to_vec()).unwrap();
    let root = engine
        .edit(
            empty,
            &[Edit {
                path: vec![b"file".to_vec()],
                expected: Entry::Absent,
                value: Entry::File {
                    blob,
                    executable: false,
                },
            }],
        )
        .unwrap();
    for (time_ns, message) in [(-1, "message"), (1, "NUL\0message")] {
        let mut revision = revision(root);
        revision.time_ns = time_ns;
        revision.message = message.into();
        let object = Object::Revision(revision.clone());
        let id = object.id().unwrap(); // Still valid canonical native metadata.
        assert_eq!(
            Object::decode(id, &object.encode().unwrap()).unwrap(),
            object
        );
        let mut plan = GitPlan::new();
        plan.tree(&store, &engine, root).unwrap();
        assert!(plan.commit(&store, id, &revision, &[]).is_err());
        assert!(plan.write(&store).is_err());
    }
    assert_eq!(object_files(dir.path()), 0);
    let tree = put(dir.path(), "tree", b"");
    let valid = raw_commit(tree);
    for raw in [
        valid.replace_bytes(b"99 -0530", b"-1 -0530"),
        valid.replace_bytes(b"123 +1245", b"-1 +1245"),
        valid.replace_bytes(b"exact message", b"NUL\0message"),
    ] {
        let oid = put(dir.path(), "commit", &raw);
        assert!(store.read_commit(oid).is_err());
    }
}

#[test]
fn required_commit_header_order_and_opaque_nuls_are_refused() {
    let (dir, store) = init();
    let tree = put(dir.path(), "tree", b"");
    let valid = raw_commit(tree);
    let text = std::str::from_utf8(&valid).unwrap();
    let (header, message) = text.split_once("\n\n").unwrap();
    let lines: Vec<_> = header.lines().collect();
    for order in [
        [1, 0, 2, 3, 4],
        [0, 2, 1, 3, 4],
        [0, 3, 1, 2, 4],
        [0, 1, 3, 2, 4],
    ] {
        let raw = format!(
            "{}\n\n{message}",
            order
                .iter()
                .map(|i| lines[*i])
                .collect::<Vec<_>>()
                .join("\n")
        );
        let oid = put(dir.path(), "commit", raw.as_bytes());
        assert!(store.read_commit(oid).is_err());
    }
    for raw in [
        valid.replace_bytes(b"continuation preserved", b"continuation \0invalid"),
        valid.replace_bytes(
            b"extra opaque",
            b"parent 1111111111111111111111111111111111111111",
        ),
        valid.replace_bytes(b"99 -0530", b"099 -0530"),
    ] {
        let oid = put(dir.path(), "commit", &raw);
        assert!(store.read_commit(oid).is_err());
    }
    // Validation checks the current commit grammar, not availability of foreign
    // ancestors: an absent parent remains an explicit frontier.
    let raw = valid.replace_bytes(
        b"\nauthor ",
        b"\nparent 1111111111111111111111111111111111111111\nauthor ",
    );
    let oid = put(dir.path(), "commit", &raw);
    assert_eq!(
        store.read_commit(oid).unwrap().parents,
        ["1111111111111111111111111111111111111111".parse().unwrap()]
    );
}

#[test]
fn git_reserved_path_aliases_are_refused_without_restricting_raw_names() {
    let (source_dir, source) = init();
    let (target_dir, target) = init();
    // hash-object's grammar checks must not be weakened by untrusted config.
    for dir in [source_dir.path(), target_dir.path()] {
        git(dir, &["config", "fsck.hasDotgit", "ignore"], &[]);
    }
    let git_blob = put(source_dir.path(), "blob", b"file");
    for name in [
        b".git".as_slice(),
        b".GiT",
        b".git.",
        b".git  .",
        b"git~1",
        b".git:ads",
        b"x\\.git",
        b".g\xe2\x80\x8cit",
        b"\xe2\x80\x8c.git",
    ] {
        let mut raw = Vec::new();
        tree_entry(&mut raw, b"100644", name, git_blob);
        let tree = put(source_dir.path(), "tree", &raw);
        let mut imported = Engine::new();
        assert!(
            source.import_source(tree, &mut imported).is_err(),
            "{name:?}"
        );
        assert_eq!(imported.object_count(), 0);
        let mut native = Engine::new();
        let empty = native.empty().unwrap();
        let blob = native.blob(b"file".to_vec()).unwrap();
        let root = native
            .edit(
                empty,
                &[Edit {
                    path: vec![name.to_vec()],
                    expected: Entry::Absent,
                    value: Entry::File {
                        blob,
                        executable: false,
                    },
                }],
            )
            .unwrap();
        let mut plan = GitPlan::new();
        assert!(plan.tree(&target, &native, root).is_err(), "{name:?}");
        assert!(plan.write(&target).is_err());
        assert_eq!(object_files(target_dir.path()), 0);
    }
    for name in [b".gitx".as_slice(), b"raw-\xff"] {
        let mut raw = Vec::new();
        tree_entry(&mut raw, b"100644", name, git_blob);
        let tree = put(source_dir.path(), "tree", &raw);
        let mut native = Engine::new();
        let root = source.import_source(tree, &mut native).unwrap();
        let mut plan = GitPlan::new();
        assert_eq!(plan.tree(&target, &native, root).unwrap(), tree);
        plan.write(&target).unwrap();
    }
    git(
        target_dir.path(),
        &["fsck", "--strict", "--no-reflogs"],
        &[],
    );
}

#[test]
fn empty_root_exports_but_cached_empty_directory_entry_refuses_without_writes() {
    let (dir, store) = init();
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let mut plan = GitPlan::new();
    let tree = plan.tree(&store, &engine, empty).unwrap();
    assert_eq!(
        tree.to_string(),
        "sha1:4b825dc642cb6eb9a060e54bf8d69288fbee4904"
    );
    let revision = revision(empty);
    let id = Object::Revision(revision.clone()).id().unwrap();
    let commit = plan.commit(&store, id, &revision, &[]).unwrap();
    assert_eq!(object_files(dir.path()), 0);
    let nested = engine
        .edit(
            empty,
            &[Edit {
                path: vec![b"empty-directory".to_vec()],
                expected: Entry::Absent,
                value: Entry::Directory(empty),
            }],
        )
        .unwrap();
    assert!(plan.tree(&store, &engine, nested).is_err());
    assert!(plan.write(&store).is_err());
    assert_eq!(object_files(dir.path()), 0);
    let mut valid = GitPlan::new();
    assert_eq!(valid.tree(&store, &engine, empty).unwrap(), tree);
    assert_eq!(valid.commit(&store, id, &revision, &[]).unwrap(), commit);
    valid.write(&store).unwrap();
    assert_eq!(store.read_commit(commit).unwrap().tree, tree);
    assert_eq!(
        git(dir.path(), &["cat-file", "tree", &bare(tree)], &[]),
        b""
    );
    git(dir.path(), &["fsck", "--strict", "--no-reflogs"], &[]);
}

#[test]
fn environment_is_scrubbed() {
    if let Some(path) = std::env::var_os("AVC_GIT_CODEC_ENV_CHILD") {
        let store = GitStore::open(Path::new(&path)).unwrap();
        let oid: GitOid = std::env::var("AVC_GIT_CODEC_ENV_OID")
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            store.read_commit(oid).unwrap().author,
            "Original Name <author@example.test>"
        );
        return;
    }
    let (dir, _) = init();
    let tree = put(dir.path(), "tree", b"");
    let oid = put(dir.path(), "commit", &raw_commit(tree));
    let marker = dir.path().join("trace-must-not-exist");
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "environment_is_scrubbed"])
        .env("AVC_GIT_CODEC_ENV_CHILD", dir.path())
        .env("AVC_GIT_CODEC_ENV_OID", oid.to_string())
        .env("GIT_DIR", "/nonexistent")
        .env("GIT_COMMON_DIR", "/nonexistent")
        .env("GIT_OBJECT_DIRECTORY", "/nonexistent")
        .env("GIT_ALTERNATE_OBJECT_DIRECTORIES", "/nonexistent")
        .env("GIT_CONFIG_COUNT", "invalid")
        .env("GIT_CONFIG_GLOBAL", "/nonexistent")
        .env("GIT_TRACE", &marker)
        .env("GIT_TRACE2_EVENT", &marker)
        .env("GIT_CONFIG_SYSTEM", "/nonexistent")
        .env("GIT_NO_REPLACE_OBJECTS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    assert!(!marker.exists());
}

// An independently assembled stock-Git tree with raw names, all supported
// modes, Git's directory sort order, shared blobs and nested empty directories.
fn portable_fixture(dir: &Path) -> (GitOid, Vec<u8>, GitOid) {
    let blob = put(dir, "blob", b"\0portable\xff\n");
    let empty = put(dir, "tree", b"");
    let mut child = Vec::new();
    tree_entry(&mut child, b"40000", b"empty", empty);
    tree_entry(&mut child, b"100755", b"exec", blob);
    let child = put(dir, "tree", &child);
    let mut raw = Vec::new();
    tree_entry(&mut raw, b"100644", b"dir.file", blob);
    tree_entry(&mut raw, b"40000", b"dir", child);
    tree_entry(&mut raw, b"40000", b"empty", empty);
    tree_entry(&mut raw, b"120000", b"link", blob);
    tree_entry(&mut raw, b"100644", b"raw-\xff", blob);
    let tree = put(dir, "tree", &raw);
    // Neither the original planner nor the writer may demand this ancestor.
    let raw = raw_commit(tree).replace_bytes(
        b"\nauthor ",
        b"\nparent 1111111111111111111111111111111111111111\nauthor ",
    );
    let commit = put(dir, "commit", &raw);
    (tree, raw, commit)
}

#[test]
fn offline_codec_and_original_plan_do_not_need_git() {
    const CASE: &str = "offline_codec_and_original_plan_do_not_need_git";
    if let Ok(fixture) = std::env::var("AVC_OFFLINE_CODEC_FIXTURE") {
        // This subprocess has no Git (or anything else) on PATH. Native
        // construction is independent of the stock-Git fixture in the parent.
        let mut engine = Engine::new();
        let empty = engine.empty().unwrap();
        let blob = engine.blob(b"\0portable\xff\n".to_vec()).unwrap();
        let file = Entry::File {
            blob,
            executable: false,
        };
        let child = engine
            .edit(
                empty,
                &[
                    Edit {
                        path: vec![b"empty".to_vec()],
                        expected: Entry::Absent,
                        value: Entry::Directory(empty),
                    },
                    Edit {
                        path: vec![b"exec".to_vec()],
                        expected: Entry::Absent,
                        value: Entry::File {
                            blob,
                            executable: true,
                        },
                    },
                ],
            )
            .unwrap();
        let root = engine
            .edit(
                empty,
                &[
                    Edit {
                        path: vec![b"dir".to_vec()],
                        expected: Entry::Absent,
                        value: Entry::Directory(child),
                    },
                    Edit {
                        path: vec![b"dir.file".to_vec()],
                        expected: Entry::Absent,
                        value: file.clone(),
                    },
                    Edit {
                        path: vec![b"empty".to_vec()],
                        expected: Entry::Absent,
                        value: Entry::Directory(empty),
                    },
                    Edit {
                        path: vec![b"link".to_vec()],
                        expected: Entry::Absent,
                        value: Entry::Symlink(blob),
                    },
                    Edit {
                        path: vec![b"raw-\xff".to_vec()],
                        expected: Entry::Absent,
                        value: file,
                    },
                ],
            )
            .unwrap();
        let oid = std::env::var("AVC_OFFLINE_CODEC_OID")
            .unwrap()
            .parse()
            .unwrap();
        let raw = std::fs::read(fixture).unwrap();
        let commit = GitCommit::decode(oid, raw.clone()).unwrap();
        let mut hashing = GitTreeHasher::new();
        assert_eq!(hashing.tree(&engine, root).unwrap(), commit.tree);
        assert_eq!(hashing.tree(&engine, root).unwrap(), commit.tree);
        assert_eq!(
            hashing.tree(&engine, child).unwrap(),
            GitTreeHasher::new().tree(&engine, child).unwrap()
        );
        let mut plan = GitPlan::new();
        assert_eq!(plan.original(&engine, root, &commit).unwrap(), oid);
        assert_eq!(plan.original(&engine, root, &commit).unwrap(), oid);
        let mut changed = raw.clone();
        *changed.last_mut().unwrap() ^= 1;
        assert!(GitCommit::decode(oid, changed).is_err());
        assert!(GitCommit::decode(commit.tree, raw).is_err());
        return;
    }
    let (dir, _) = init();
    let (_, raw, oid) = portable_fixture(dir.path());
    let fixture = dir.path().join("raw-commit");
    std::fs::write(&fixture, raw).unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", CASE, "--nocapture"])
        .env("AVC_OFFLINE_CODEC_FIXTURE", fixture)
        .env("AVC_OFFLINE_CODEC_OID", oid.to_string())
        .env("PATH", dir.path().join("no-executables"))
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn original_rehydrates_exact_closure_even_when_commit_already_exists() {
    let (dir, source) = init();
    let (tree, raw, oid) = portable_fixture(dir.path());
    let mut engine = Engine::new();
    let root = source.import_source(tree, &mut engine).unwrap();
    let original = GitCommit::decode(oid, raw.clone()).unwrap();
    let (target_dir, target) = init();
    let mut plan = GitPlan::new();
    assert_eq!(plan.original(&engine, root, &original).unwrap(), oid);
    assert_eq!(object_files(target_dir.path()), 0);
    assert_eq!(target.read_commit_optional(oid).unwrap(), None);
    // Install only the original commit. Its absent tree must still be planned.
    assert_eq!(put(target_dir.path(), "commit", &raw), oid);
    plan.write(&target).unwrap();
    assert_eq!(target.read_commit_optional(oid).unwrap(), Some(original));
    assert_eq!(target.read_commit(oid).unwrap().raw, raw);
    // cat-file supplies a virtual empty tree, but a complete stored closure
    // must also materialize it for stock Git connectivity/fsck checks.
    assert_eq!(object_files(target_dir.path()), 5); // blob, 3 trees, commit
    let mut imported = Engine::new();
    assert_eq!(target.import_source(tree, &mut imported).unwrap(), root);
    plan.write(&target).unwrap();
    assert_eq!(object_files(target_dir.path()), 5);
    let foreign = "1111111111111111111111111111111111111111".parse().unwrap();
    assert_eq!(target.read_commit_optional(foreign).unwrap(), None);
}

#[test]
fn corrupt_physical_empty_tree_is_not_hidden_by_gits_virtual_object() {
    let (dir, store) = init();
    let tree: GitOid = "4b825dc642cb6eb9a060e54bf8d69288fbee4904".parse().unwrap();
    let evil = put(dir.path(), "blob", b"corrupt empty tree slot");
    let object_path = |oid| {
        let hex = bare(oid);
        dir.path().join("objects").join(&hex[..2]).join(&hex[2..])
    };
    std::fs::create_dir_all(object_path(tree).parent().unwrap()).unwrap();
    std::fs::copy(object_path(evil), object_path(tree)).unwrap();
    // Confirm the dangerous stock-Git behavior independently of our reader.
    assert_eq!(
        git(dir.path(), &["cat-file", "tree", &bare(tree)], &[]),
        b""
    );
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let mut plan = GitPlan::new();
    plan.tree(&store, &engine, empty).unwrap();
    let rev = revision(empty);
    let id = Object::Revision(rev.clone()).id().unwrap();
    plan.commit(&store, id, &rev, &[]).unwrap();
    let before = object_files(dir.path());
    assert!(plan.write(&store).is_err());
    assert_eq!(object_files(dir.path()), before);
}

#[test]
fn original_present_commit_does_not_hide_corrupt_source_objects() {
    let (dir, source) = init();
    let (tree, raw, oid) = portable_fixture(dir.path());
    let mut engine = Engine::new();
    let root = source.import_source(tree, &mut engine).unwrap();
    let original = GitCommit::decode(oid, raw.clone()).unwrap();
    let (target_dir, target) = init();
    put(target_dir.path(), "commit", &raw);
    let evil = put(target_dir.path(), "blob", b"not the planned tree");
    let object_path = |oid| {
        let hex = bare(oid);
        target_dir
            .path()
            .join("objects")
            .join(&hex[..2])
            .join(&hex[2..])
    };
    std::fs::create_dir_all(object_path(tree).parent().unwrap()).unwrap();
    std::fs::copy(object_path(evil), object_path(tree)).unwrap();
    let before = object_files(target_dir.path());
    assert_eq!(target.read_commit_optional(oid).unwrap(), Some(original));
    let original = GitCommit::decode(oid, raw).unwrap();
    let mut plan = GitPlan::new();
    plan.original(&engine, root, &original).unwrap();
    assert!(plan.write(&target).is_err());
    assert_eq!(object_files(target_dir.path()), before);
    // A wrong-type or corrupt commit slot is also an error, never missing.
    std::fs::remove_file(object_path(oid)).unwrap();
    std::fs::copy(object_path(evil), object_path(oid)).unwrap();
    assert!(target.read_commit_optional(oid).is_err());
}

#[cfg(unix)]
#[test]
fn optional_read_requires_exact_missing_framing_and_successful_exit() {
    use std::os::unix::fs::PermissionsExt;
    const CASE: &str = "optional_read_requires_exact_missing_framing_and_successful_exit";
    let oid: GitOid = "1111111111111111111111111111111111111111".parse().unwrap();
    if let Ok(dir) = std::env::var("AVC_OPTIONAL_READ_CHILD") {
        let store = GitStore::open(Path::new(&dir)).unwrap();
        let result = store.read_commit_optional(oid);
        if std::env::var("AVC_OPTIONAL_READ_MISSING").unwrap() == "yes" {
            assert_eq!(result.unwrap(), None);
        } else {
            assert!(
                result.is_err(),
                "invalid protocol response accepted: {result:?}"
            );
        }
        return;
    }
    let (dir, _) = init();
    let real_git = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|path| path.join("git"))
        .find(|path| path.is_file())
        .unwrap()
        .canonicalize()
        .unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    for (response, status, missing, diagnostic) in [
        (format!("{} missing\n", bare(oid)), 0, true, false),
        (format!("{} missing\n", bare(oid)), 1, false, false),
        (format!("{} missing", bare(oid)), 0, false, false),
        (format!("{} missing\nextra", bare(oid)), 0, false, false),
        (format!("{} missing\n\n", bare(oid)), 0, false, false),
        (
            "2222222222222222222222222222222222222222 missing\n".into(),
            0,
            false,
            false,
        ),
        (format!("{} tree 0\n\n", bare(oid)), 0, false, false),
        (format!("{} commit 3\nx", bare(oid)), 0, false, false),
        (format!("{} commit 0\n\n", bare(oid)), 0, false, false),
        (format!("{} missing\n", bare(oid)), 0, false, true),
    ] {
        let script = bin.join("git");
        // Exceed pipe capacity before stdout: a sequential stderr reader would
        // deadlock, and discarding stderr would wrongly accept the missing OID.
        let diagnostics = if diagnostic {
            "diagnostic".repeat(32 * 1024)
        } else {
            String::new()
        };
        std::fs::write(&script, format!(
            "#!/bin/sh\ncase \"$*\" in\n  *cat-file*) printf '%s' '{diagnostics}' >&2; printf '%s' '{response}'; exit {status};;\n  *) exec '{}' \"$@\";;\nesac\n", real_git.display()
        )).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", CASE, "--nocapture"])
            .env("PATH", &bin)
            .env("AVC_OPTIONAL_READ_CHILD", dir.path())
            .env(
                "AVC_OPTIONAL_READ_MISSING",
                if missing { "yes" } else { "no" },
            )
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "response {response:?}");
    }
}

#[test]
fn original_cache_does_not_grant_native_empty_directory_representability() {
    let (dir, source) = init();
    let (tree, raw, oid) = portable_fixture(dir.path());
    let mut engine = Engine::new();
    let root = source.import_source(tree, &mut engine).unwrap();
    let original = GitCommit::decode(oid, raw).unwrap();
    let (target_dir, target) = init();
    let mut plan = GitPlan::new();
    plan.original(&engine, root, &original).unwrap();
    assert!(plan.tree(&target, &engine, root).is_err());
    assert!(plan.write(&target).is_err());
    assert_eq!(object_files(target_dir.path()), 0);
    let empty = engine.empty().unwrap();
    let mut wrong = GitPlan::new();
    assert!(wrong.original(&engine, empty, &original).is_err());
    assert!(wrong.write(&target).is_err());
    assert_eq!(object_files(target_dir.path()), 0);
}

#[test]
fn original_stock_git_admission_failure_prevents_all_writes() {
    let (source_dir, _) = init();
    let blob = put(source_dir.path(), "blob", b"payload");
    let mut raw = Vec::new();
    tree_entry(&mut raw, b"100644", b".git", blob);
    let tree = put(source_dir.path(), "tree", &raw);
    let raw = raw_commit(tree);
    let oid = put(source_dir.path(), "commit", &raw);
    let commit = GitCommit::decode(oid, raw).unwrap();
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let blob = engine.blob(b"payload".to_vec()).unwrap();
    let root = engine
        .edit(
            empty,
            &[Edit {
                path: vec![b".git".to_vec()],
                expected: Entry::Absent,
                value: Entry::File {
                    blob,
                    executable: false,
                },
            }],
        )
        .unwrap();
    let mut plan = GitPlan::new();
    plan.original(&engine, root, &commit).unwrap();
    let (target_dir, target) = init();
    assert!(plan.write(&target).is_err());
    assert_eq!(object_files(target_dir.path()), 0);
}

#[test]
fn offline_source_hashing_budgets_expansion_even_with_cached_children() {
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let blob = engine.blob(b"shared".to_vec()).unwrap();
    let mut root = engine
        .edit(
            empty,
            &[Edit {
                path: vec![b"leaf".to_vec()],
                expected: Entry::Absent,
                value: Entry::File {
                    blob,
                    executable: false,
                },
            }],
        )
        .unwrap();
    let mut hashing = GitTreeHasher::new();
    for _ in 0..15 {
        hashing.tree(&engine, root).unwrap();
        root = engine
            .edit(
                empty,
                &[
                    Edit {
                        path: vec![b"a".to_vec()],
                        expected: Entry::Absent,
                        value: Entry::Directory(root),
                    },
                    Edit {
                        path: vec![b"b".to_vec()],
                        expected: Entry::Absent,
                        value: Entry::Directory(root),
                    },
                ],
            )
            .unwrap();
    }
    // Only 17 unique trees; the next root expands beyond 100,000 entries.
    hashing.tree(&engine, root).unwrap();
    root = engine
        .edit(
            empty,
            &[
                Edit {
                    path: vec![b"a".to_vec()],
                    expected: Entry::Absent,
                    value: Entry::Directory(root),
                },
                Edit {
                    path: vec![b"b".to_vec()],
                    expected: Entry::Absent,
                    value: Entry::Directory(root),
                },
            ],
        )
        .unwrap();
    assert!(hashing.tree(&engine, root).is_err());
    assert!(hashing.tree(&engine, empty).is_err());
}

#[test]
fn explicit_admin_path_and_sha256_refusal() {
    let (dir, _) = init();
    let child = dir.path().join("not-a-repository");
    std::fs::create_dir(&child).unwrap();
    assert!(GitStore::open(&child).is_err());
    let sha256 = tempfile::tempdir().unwrap();
    git(
        sha256.path(),
        &["init", "--bare", "--quiet", "--object-format=sha256"],
        &[],
    );
    assert!(GitStore::open(sha256.path()).is_err());
}
