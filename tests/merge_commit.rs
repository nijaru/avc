//! Merge authoring consumes an already selected Source; composition is a separate act.
use avc::{
    Entry, Id, REVISION_KIND,
    engine::{Edit, Engine, MergeBudget},
    repository::{Error, Operation, Receipt, RefName, Repository, SelectionReceipt},
};
use rusqlite::Connection;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, Barrier},
};

fn put(engine: &mut Engine, root: Id, name: &str, bytes: &[u8]) -> Id {
    let path = vec![name.as_bytes().to_vec()];
    let expected = engine.read(root, &path).unwrap();
    let blob = engine.blob(bytes.to_vec()).unwrap();
    engine
        .edit(
            root,
            &[Edit {
                path,
                expected,
                value: Entry::File {
                    blob,
                    executable: false,
                },
            }],
        )
        .unwrap()
}
fn select(repo: &mut Repository, engine: &Engine, root: Id, key: &str) {
    repo.commit_source(engine, root, repo.status().unwrap().generation, key)
        .unwrap();
}
fn author(repo: &mut Repository, key: &str) -> Id {
    repo.commit(
        "Contract Author <contract@example.test>",
        key,
        repo.status().unwrap().generation,
        key,
    )
    .unwrap()
    .after
    .revision
    .unwrap()
}
struct Branches {
    repo: Repository,
    dir: tempfile::TempDir,
    engine: Engine,
    base: Id,
    left: Id,
    right: Id,
    base_root: Id,
    left_root: Id,
    right_root: Id,
}
impl Branches {
    fn new(overlap: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = Repository::init(dir.path()).unwrap();
        let mut engine = Engine::new();
        let empty = engine.empty().unwrap();
        let base_root = put(&mut engine, empty, "a", b"base\0a");
        let base_root = put(&mut engine, base_root, "b", b"base\0b");
        let left_root = put(&mut engine, base_root, "a", b"left\0a");
        let right_root = put(
            &mut engine,
            base_root,
            if overlap { "a" } else { "b" },
            b"right\0value",
        );
        select(&mut repo, &engine, base_root, "base-source");
        let base = author(&mut repo, "base");
        select(&mut repo, &engine, left_root, "left-source");
        let left = author(&mut repo, "left");
        repo.select_revision(base, repo.status().unwrap().generation, "select-base")
            .unwrap();
        select(&mut repo, &engine, right_root, "right-source");
        let right = author(&mut repo, "right");
        repo.select_revision(left, repo.status().unwrap().generation, "select-left")
            .unwrap();
        Self {
            dir,
            repo,
            engine,
            base,
            left,
            right,
            base_root,
            left_root,
            right_root,
        }
    }
    fn compose(&mut self) -> Id {
        let merged = self
            .engine
            .merge(
                self.base_root,
                self.left_root,
                self.right_root,
                MergeBudget::default(),
            )
            .unwrap();
        select(&mut self.repo, &self.engine, merged.root, "composed-source");
        merged.root
    }
    fn merge(&mut self, key: &str) -> SelectionReceipt {
        self.repo
            .commit_merge(
                self.right,
                "Contract Author",
                "merge",
                self.repo.status().unwrap().generation,
                key,
            )
            .unwrap()
    }
}
fn database(project: &Path) -> Connection {
    // Only SQLite opens DB/WAL/SHM, including deliberate corruption probes.
    Connection::open(project.join(".avc/store.sqlite")).unwrap()
}
fn rows(project: &Path) -> Vec<(Vec<u8>, i64, Vec<u8>)> {
    database(project)
        .prepare("SELECT id, length, payload FROM objects ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}
fn damage(project: &Path, id: Id, remove: bool) {
    let db = database(project);
    db.pragma_update(None, "foreign_keys", false).unwrap();
    db.execute_batch(
        "DROP TRIGGER IF EXISTS objects_no_update; DROP TRIGGER IF EXISTS objects_no_delete;",
    )
    .unwrap();
    let sql = if remove {
        "DELETE FROM objects WHERE id = ?1"
    } else {
        "UPDATE objects SET payload = zeroblob(length) WHERE id = ?1"
    };
    assert_eq!(
        db.execute(sql, [id.to_bytes().unwrap().as_slice()])
            .unwrap(),
        1
    );
}
fn rejected(repo: &mut Repository, project: &Path, parent: Id, key: &str) {
    let before = repo.status().unwrap();
    let retained = rows(project);
    assert!(
        repo.commit_merge(parent, "Contract Author", "merge", before.generation, key)
            .is_err(),
        "accepted {key}"
    );
    assert_eq!(repo.status().unwrap(), before);
    assert_eq!(
        rows(project),
        retained,
        "rejected request published objects"
    );
    assert!(repo.receipt(key).unwrap().is_none());
}
fn blob(engine: &Engine, root: Id, name: &str) -> Id {
    let Entry::File { blob, .. } = engine.read(root, &[name.as_bytes().to_vec()]).unwrap() else {
        panic!("file")
    };
    blob
}
fn git(dir: &Path, args: &[&str], input: &[u8]) -> String {
    let mut child = Command::new("git")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .current_dir(dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}
fn check_python(repo: &Repository, head: Id, parents: &[Id], directory: &Path) {
    if std::env::var_os("AVC_REFERENCE_CHECK").is_none() {
        return;
    }
    let engine = repo.load_root(head).unwrap();
    let objects = engine
        .objects()
        .map(|(id, object)| {
            let payload: String = object
                .encode()
                .unwrap()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            serde_json::json!({"id": id.to_string(), "payload": payload})
        })
        .collect::<Vec<_>>();
    let corpus = directory.join("merge-history.json");
    fs::write(
        &corpus,
        serde_json::to_vec(&serde_json::json!({"objects": objects, "roots": [head.to_string()]}))
            .unwrap(),
    )
    .unwrap();
    // The independent decoder both walks every parent and checks their order,
    // rather than merely accepting the Rust first-parent log as a history oracle.
    let script = r#"
import json, sys
sys.path.insert(0, 'reference')
import check
with open(sys.argv[1]) as f: data = json.load(f)
check.verify_corpus(data)
record = next(o for o in data['objects'] if o['id'] == sys.argv[2])
node = check.decode_node(5, bytes.fromhex(record['payload']))
assert [check.spelling(p) for p in node[1]] == sys.argv[3:]
"#;
    let output = Command::new("uv")
        .args(["run", "--with", "blake3==1.0.8", "python", "-c", script])
        .arg(corpus)
        .arg(head.to_string())
        .args(parents.iter().map(ToString::to_string))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "independent ordered history check: {output:?}"
    );
}

#[test]
fn composed_source_authors_ordered_history_and_git_candidate_without_ref_or_file_effects() {
    let mut f = Branches::new(false);
    let merged_root = f.compose();
    let live = f.dir.path().join("live");
    fs::write(&live, b"not selected; do not capture or overwrite").unwrap();
    let name: RefName = "saved".parse().unwrap();
    f.repo.set_reference(&name, f.left, 0, "save-left").unwrap();
    let refs = f.repo.references(None, 20).unwrap();
    git(f.dir.path(), &["init"], b"");
    let tree = git(f.dir.path(), &["mktree"], b"");
    let raw = format!(
        "tree {tree}\nauthor Guard <g@example.test> 1700000000 +0000\ncommitter Guard <g@example.test> 1700000000 +0000\n\nuntouched Git ref\n"
    );
    let guard = git(
        f.dir.path(),
        &["hash-object", "-w", "-t", "commit", "--stdin"],
        raw.as_bytes(),
    );
    git(
        f.dir.path(),
        &["update-ref", "refs/heads/guard", &guard],
        b"",
    );
    let local_git_refs = git(f.dir.path(), &["for-each-ref"], b"");
    let local_git_head = fs::read(f.dir.path().join(".git/HEAD")).unwrap();
    let before = f.repo.status().unwrap();
    let receipt = f
        .repo
        .commit_merge(
            f.right,
            "Contract Author <contract@example.test>",
            "merge",
            before.generation,
            "merge",
        )
        .unwrap();
    assert_eq!(
        receipt.operation,
        Operation::MergeCommit { parent: f.right }
    );
    assert_eq!(receipt.before, before);
    assert_eq!(receipt.after.root, merged_root);
    assert_eq!(receipt.after.generation, before.generation + 1);
    let head = receipt.after.revision.unwrap();
    let revision = f.repo.revision(head).unwrap();
    assert_eq!(revision.parents, vec![f.left, f.right]);
    assert_eq!(revision.root, merged_root);
    assert_eq!(revision.author, "Contract Author <contract@example.test>");
    assert_eq!(revision.message, "merge");
    for parent in [f.left, f.right] {
        assert_ne!(revision.change, f.repo.revision(parent).unwrap().change);
    }
    assert_eq!(
        f.engine.read(merged_root, &[b"a".to_vec()]).unwrap(),
        f.engine.read(f.left_root, &[b"a".to_vec()]).unwrap()
    );
    assert_eq!(
        f.engine.read(merged_root, &[b"b".to_vec()]).unwrap(),
        f.engine.read(f.right_root, &[b"b".to_vec()]).unwrap()
    );
    check_python(&f.repo, head, &[f.left, f.right], f.dir.path());
    let destination = tempfile::tempdir().unwrap();
    git(destination.path(), &["init", "--bare"], b"");
    let git_refs = git(destination.path(), &["for-each-ref"], b"");
    let git_head = fs::read(destination.path().join("HEAD")).unwrap();
    let left = f.repo.export_git(destination.path(), f.left).unwrap();
    let right = f.repo.export_git(destination.path(), f.right).unwrap();
    let candidate = f.repo.export_git(destination.path(), head).unwrap();
    let raw = git(
        destination.path(),
        &[
            "cat-file",
            "commit",
            candidate.to_string().strip_prefix("sha1:").unwrap(),
        ],
        b"",
    );
    let ordered = raw
        .lines()
        .filter_map(|line| line.strip_prefix("parent "))
        .collect::<Vec<_>>();
    assert_eq!(
        ordered,
        vec![
            left.to_string().strip_prefix("sha1:").unwrap(),
            right.to_string().strip_prefix("sha1:").unwrap()
        ]
    );
    assert_eq!(git(destination.path(), &["for-each-ref"], b""), git_refs);
    assert_eq!(fs::read(destination.path().join("HEAD")).unwrap(), git_head);
    assert_eq!(f.repo.references(None, 20).unwrap(), refs);
    assert_eq!(git(f.dir.path(), &["for-each-ref"], b""), local_git_refs);
    assert_eq!(
        fs::read(f.dir.path().join(".git/HEAD")).unwrap(),
        local_git_head
    );
    assert_eq!(
        fs::read(live).unwrap(),
        b"not selected; do not capture or overwrite"
    );
    assert!(!f.dir.path().join("a").exists());
    assert!(!f.dir.path().join("b").exists());
    drop(f.repo);
    let reopened = Repository::open(f.dir.path()).unwrap();
    assert_eq!(reopened.status().unwrap(), receipt.after);
    assert_eq!(
        reopened.receipt("merge").unwrap(),
        Some(Receipt::Selection(receipt))
    );
}

#[test]
fn unchanged_ancestor_and_unrelated_parents_do_not_compute_source_or_fast_forward() {
    let mut f = Branches::new(false);
    let selected = f.repo.status().unwrap();
    let merge = f.merge("unchanged");
    assert_eq!(merge.after.root, selected.root);
    assert_eq!(
        f.repo
            .revision(merge.after.revision.unwrap())
            .unwrap()
            .parents,
        vec![f.left, f.right]
    );
    let ancestor = f
        .repo
        .commit_merge(
            f.base,
            "Contract Author",
            "ancestor",
            merge.after.generation,
            "ancestor",
        )
        .unwrap();
    assert_eq!(ancestor.after.root, selected.root);
    assert_eq!(
        f.repo
            .revision(ancestor.after.revision.unwrap())
            .unwrap()
            .parents,
        vec![merge.after.revision.unwrap(), f.base]
    );
    let other = tempfile::tempdir().unwrap();
    let mut independent = Repository::init(other.path()).unwrap();
    let unrelated = author(&mut independent, "independent-birth");
    independent
        .undo("independent-birth", 1, "return-unborn")
        .unwrap();
    let second_birth = author(&mut independent, "second-birth");
    assert!(
        independent
            .revision(second_birth)
            .unwrap()
            .parents
            .is_empty()
    );
    let before = independent.status().unwrap();
    let authored = independent
        .commit_merge(
            unrelated,
            "Contract Author",
            "unrelated",
            before.generation,
            "unrelated",
        )
        .unwrap();
    assert_eq!(authored.after.root, before.root);
    assert_eq!(
        independent
            .revision(authored.after.revision.unwrap())
            .unwrap()
            .parents,
        vec![second_birth, unrelated]
    );
}

#[test]
fn typed_conflicts_are_authored_without_resolution() {
    let mut f = Branches::new(true);
    let root = f.compose();
    let Entry::Conflict(conflict) = f.engine.read(root, &[b"a".to_vec()]).unwrap() else {
        panic!("composition must conflict")
    };
    let receipt = f.merge("conflicted-merge");
    assert_eq!(receipt.after.root, root);
    let loaded = f.repo.load_root(receipt.after.revision.unwrap()).unwrap();
    assert_eq!(
        loaded.object(conflict).unwrap(),
        f.engine.object(conflict).unwrap()
    );
    loaded.verify(root, Default::default()).unwrap();
}

#[test]
fn undo_restores_exact_dirty_candidate_amend_preserves_pair_and_change_and_rebase_refuses_merge() {
    let mut f = Branches::new(false);
    let composed = f.compose();
    let before = f.repo.status().unwrap();
    assert_ne!(composed, f.left_root);
    let receipt = f.merge("merge");
    let id = receipt.after.revision.unwrap();
    let original = f.repo.revision(id).unwrap();
    let undone = f
        .repo
        .undo("merge", receipt.after.generation, "undo-merge")
        .unwrap();
    assert_eq!(
        (undone.after.root, undone.after.revision),
        (before.root, before.revision)
    );
    assert_eq!(undone.after.generation, receipt.after.generation + 1);
    let redone = f
        .repo
        .undo("undo-merge", undone.after.generation, "redo-merge")
        .unwrap();
    let retained = rows(f.dir.path());
    assert!(
        f.repo
            .rebase(f.base, redone.after.generation, "no-rebase")
            .is_err()
    );
    assert_eq!(f.repo.status().unwrap(), redone.after);
    assert_eq!(rows(f.dir.path()), retained);
    assert!(f.repo.receipt("no-rebase").unwrap().is_none());
    select(&mut f.repo, &f.engine, f.right_root, "amend-source");
    let amended = f
        .repo
        .amend(
            "New Author",
            "amended merge",
            f.repo.status().unwrap().generation,
            "amend",
        )
        .unwrap();
    let revision = f.repo.revision(amended.after.revision.unwrap()).unwrap();
    assert_eq!(revision.parents, vec![f.left, f.right]);
    assert_eq!(revision.change, original.change);
    assert_eq!(revision.root, f.right_root);
    assert_eq!(f.repo.revision(id).unwrap(), original);
}

#[test]
fn invalid_birth_duplicate_unknown_and_nonrevision_parent_requests_publish_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut unborn = Repository::init(dir.path()).unwrap();
    let other = tempfile::tempdir().unwrap();
    let mut known = Repository::init(other.path()).unwrap();
    let parent = author(&mut known, "parent");
    let mut history = Vec::new();
    known.export_history(&mut history).unwrap();
    unborn
        .import_history(history.as_slice(), "known-parent")
        .unwrap();
    unborn.undo("known-parent", 1, "unborn-again").unwrap();
    rejected(&mut unborn, dir.path(), parent, "requires-head");
    let mut f = Branches::new(false);
    let unknown: Id = format!("avc1-k{REVISION_KIND:02x}-{}", "00".repeat(32))
        .parse()
        .unwrap();
    let wrong_blob = blob(&f.engine, f.left_root, "a");
    for (parent, key) in [
        (f.left, "duplicate"),
        (unknown, "unknown"),
        (f.left_root, "map"),
        (wrong_blob, "blob"),
    ] {
        rejected(&mut f.repo, f.dir.path(), parent, key);
    }
}

#[test]
fn parents_require_metadata_and_direct_presence_not_historical_payload_health() {
    for case in [
        "head-metadata",
        "parent-metadata",
        "head-root",
        "parent-root",
        "direct-parent",
    ] {
        let mut f = Branches::new(false);
        f.compose();
        let (id, remove) = match case {
            "head-metadata" => (f.left, false),
            "parent-metadata" => (f.right, false),
            "head-root" => (f.left_root, true),
            "parent-root" => (f.right_root, true),
            "direct-parent" => (f.base, true),
            _ => unreachable!(),
        };
        damage(f.dir.path(), id, remove);
        rejected(&mut f.repo, f.dir.path(), f.right, case);
    }
    let mut f = Branches::new(false);
    // The selected closure is independent of both parents' historical blobs.
    let empty = f.engine.empty().unwrap();
    let selected = put(&mut f.engine, empty, "candidate", b"chosen exact bytes");
    select(&mut f.repo, &f.engine, selected, "independent-source");
    for id in [
        blob(&f.engine, f.left_root, "a"),
        blob(&f.engine, f.right_root, "b"),
        f.base,
    ] {
        damage(f.dir.path(), id, false);
    }
    let receipt = f.merge("metadata-only-parents");
    assert_eq!(receipt.after.root, selected);
    assert_eq!(
        f.repo
            .revision(receipt.after.revision.unwrap())
            .unwrap()
            .parents,
        vec![f.left, f.right]
    );
}

#[test]
fn full_selected_source_closure_is_required_even_with_valid_parent_metadata() {
    for remove in [false, true] {
        let mut f = Branches::new(false);
        let empty = f.engine.empty().unwrap();
        let subtree = put(&mut f.engine, empty, "file", b"selected descendant");
        let selected = f
            .engine
            .edit(
                empty,
                &[Edit {
                    path: vec![b"nested".to_vec()],
                    expected: Entry::Absent,
                    value: Entry::Directory(subtree),
                }],
            )
            .unwrap();
        select(&mut f.repo, &f.engine, selected, "dirty-source");
        damage(f.dir.path(), blob(&f.engine, subtree, "file"), remove);
        rejected(&mut f.repo, f.dir.path(), f.right, "bad-selected-closure");
    }
}

#[test]
fn global_request_identity_includes_parent_generation_author_message_and_operation_kind() {
    let mut f = Branches::new(false);
    let before = f.repo.status().unwrap();
    let receipt = f.merge("merge");
    for (parent, author, message, generation) in [
        (f.base, "Contract Author", "merge", before.generation),
        (f.right, "Other", "merge", before.generation),
        (f.right, "Contract Author", "different", before.generation),
        (f.right, "Contract Author", "merge", before.generation + 1),
        (f.right, "", "merge", before.generation),
    ] {
        assert!(matches!(
            f.repo
                .commit_merge(parent, author, message, generation, "merge"),
            Err(Error::RequestKeyReuse(_))
        ));
    }
    assert!(matches!(
        f.repo
            .commit("Contract Author", "merge", before.generation, "merge"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert!(matches!(
        f.repo
            .amend("Contract Author", "merge", before.generation, "merge"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert!(matches!(
        f.repo
            .commit_source(&f.engine, before.root, before.generation, "merge"),
        Err(Error::RequestKeyReuse(_))
    ));
    let name: RefName = "saved".parse().unwrap();
    assert!(matches!(
        f.repo.set_reference(&name, f.right, 0, "merge"),
        Err(Error::RequestKeyReuse(_))
    ));
    f.repo.set_reference(&name, f.right, 0, "ref-key").unwrap();
    for key in ["left", "left-source", "ref-key"] {
        assert!(matches!(
            f.repo
                .commit_merge(f.right, "Contract Author", "merge", before.generation, key),
            Err(Error::RequestKeyReuse(_))
        ));
    }
    assert_eq!(f.repo.status().unwrap(), receipt.after);
}

#[test]
fn exact_replay_precedes_staleness_source_ref_and_parent_health_but_needs_own_revision_metadata() {
    let mut f = Branches::new(false);
    let before = f.repo.status().unwrap();
    let receipt = f.merge("merge");
    let own = receipt.after.revision.unwrap();
    select(&mut f.repo, &f.engine, f.right_root, "later-source");
    let name: RefName = "saved".parse().unwrap();
    f.repo.set_reference(&name, f.left, 0, "save").unwrap();
    f.repo.set_reference(&name, f.right, 1, "move").unwrap();
    let later = f.repo.status().unwrap();
    for id in [f.left, f.right, blob(&f.engine, f.right_root, "b")] {
        damage(f.dir.path(), id, false);
    }
    assert_eq!(
        f.repo
            .commit_merge(
                f.right,
                "Contract Author",
                "merge",
                before.generation,
                "merge"
            )
            .unwrap(),
        receipt
    );
    assert_eq!(f.repo.status().unwrap(), later);
    // Generic journal reads do not reconstruct this operation from parent payloads
    // or even the authored payload. Author retries, unlike receipt reads, do.
    damage(f.dir.path(), own, false);
    assert_eq!(
        f.repo.receipt("merge").unwrap(),
        Some(Receipt::Selection(receipt.clone()))
    );
    assert!(
        f.repo
            .commit_merge(
                f.right,
                "Contract Author",
                "merge",
                before.generation,
                "merge"
            )
            .is_err()
    );
    assert_eq!(f.repo.status().unwrap(), later);
}

#[test]
fn concurrent_same_key_reconciles_one_identity_and_distinct_keys_have_one_cas_winner() {
    for identical in [true, false] {
        let f = Branches::new(false);
        let generation = f.repo.status().unwrap().generation;
        let baseline = rows(f.dir.path()).len();
        let barrier = Arc::new(Barrier::new(2));
        let workers = (0..2)
            .map(|i| {
                let mut repo = Repository::open(f.dir.path()).unwrap();
                let barrier = barrier.clone();
                let parent = f.right;
                std::thread::spawn(move || {
                    barrier.wait();
                    repo.commit_merge(
                        parent,
                        "Contract Author",
                        "merge",
                        generation,
                        if identical || i == 0 { "one" } else { "two" },
                    )
                })
            })
            .collect::<Vec<_>>();
        let results = workers
            .into_iter()
            .map(|w| w.join().unwrap())
            .collect::<Vec<_>>();
        if identical {
            assert_eq!(results[0].as_ref().unwrap(), results[1].as_ref().unwrap());
        } else {
            assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
            assert_eq!(results.iter().filter(|r| matches!(r, Err(Error::Stale { expected, actual }) if *expected == generation && *actual == generation + 1)).count(), 1);
        }
        assert_eq!(f.repo.status().unwrap().generation, generation + 1);
        assert_eq!(rows(f.dir.path()).len(), baseline + 1);
    }
}

#[test]
fn imported_git_anchors_are_valid_parents_without_origin_custody_lookup() {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(dir.path()).unwrap();
    let foreign = tempfile::tempdir().unwrap();
    git(foreign.path(), &["init", "--bare"], b"");
    let tree = git(foreign.path(), &["mktree"], b"");
    let raw = format!(
        "tree {tree}\nauthor Foreign <f@example.test> 1700000000 +0000\ncommitter Foreign <f@example.test> 1700000000 +0000\n\nforeign\n"
    );
    let oid = git(
        foreign.path(),
        &["hash-object", "-w", "-t", "commit", "--stdin"],
        raw.as_bytes(),
    );
    let anchor = repo
        .import_git(foreign.path(), oid.parse().unwrap(), "foreign")
        .unwrap()
        .after
        .revision
        .unwrap();
    repo.undo("foreign", 1, "unborn").unwrap();
    let native = author(&mut repo, "native");
    repo.select_revision(anchor, repo.status().unwrap().generation, "anchor-head")
        .unwrap();
    // Origin custody is a separate boundary from canonical revision parenthood.
    let db = database(dir.path());
    db.execute_batch(
        "DROP TRIGGER git_origins_no_update; UPDATE git_origins SET checksum = zeroblob(32);",
    )
    .unwrap();
    drop(db);
    assert!(repo.git_origin(anchor).is_err());
    let before = repo.status().unwrap();
    let from_anchor = repo
        .commit_merge(
            native,
            "Contract Author",
            "foreign head",
            before.generation,
            "foreign-head",
        )
        .unwrap();
    assert_eq!(
        repo.revision(from_anchor.after.revision.unwrap())
            .unwrap()
            .parents,
        vec![anchor, native]
    );
    repo.select_revision(native, from_anchor.after.generation, "return-native")
        .unwrap();
    let merge = repo
        .commit_merge(
            anchor,
            "Contract Author",
            "foreign second parent",
            repo.status().unwrap().generation,
            "foreign-parent",
        )
        .unwrap();
    assert_eq!(
        repo.revision(merge.after.revision.unwrap())
            .unwrap()
            .parents,
        vec![native, anchor]
    );
}
