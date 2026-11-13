//! Independent public contracts for atomic, explicit-base linear-stack replay.
use avc::{
    Conflict, Entry, Id, Node, Object, Revision,
    engine::{Edit, Engine},
    repository::{
        Error, Operation, Receipt, RefName, Repository, RevisionRewrite, SelectionReceipt,
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, Barrier},
};

#[derive(Default)]
struct Graph {
    engine: Engine,
    revisions: BTreeMap<Id, Object>,
}
impl Graph {
    fn source(&mut self, files: &[(&str, &[u8])]) -> Id {
        let empty = self.engine.empty().unwrap();
        let edits = files
            .iter()
            .map(|(name, bytes)| Edit {
                path: vec![name.as_bytes().to_vec()],
                expected: Entry::Absent,
                value: Entry::File {
                    blob: self.engine.blob(bytes.to_vec()).unwrap(),
                    executable: false,
                },
            })
            .collect::<Vec<_>>();
        self.engine.edit(empty, &edits).unwrap()
    }
    fn revision(&mut self, root: Id, parents: &[Id], change: u8) -> Id {
        let object = Object::Revision(Revision {
            root,
            parents: parents.to_vec(),
            change: [change; 16],
            author: format!("Author {change} <a{change}@example.test>"),
            message: format!("change {change}\n\nExact original message.\n"),
            time_ns: -9_876_543_210 + i64::from(change),
            timezone_minutes: -210 + i16::from(change),
        });
        let id = object.id().unwrap();
        self.revisions.insert(id, object);
        id
    }
    fn object(&self, id: Id) -> &Object {
        self.revisions
            .get(&id)
            .unwrap_or_else(|| self.engine.object(id).unwrap())
    }
    fn root(&self, id: Id) -> Id {
        let Object::Revision(r) = self.object(id) else {
            panic!("revision fixture")
        };
        r.root
    }
    fn bundle(&self, head: Id) -> Vec<u8> {
        // Build a literal public history stream, excluding construction debris.
        fn entry(entry: &Entry, pending: &mut Vec<Id>) {
            match entry {
                Entry::Absent => {}
                Entry::File { blob, .. } | Entry::Symlink(blob) => pending.push(*blob),
                Entry::Directory(id) | Entry::Conflict(id) => pending.push(*id),
            }
        }
        let mut pending = vec![head];
        let mut records = BTreeMap::new();
        while let Some(id) = pending.pop() {
            if records.contains_key(&id) {
                continue;
            }
            let object = self.object(id);
            match object {
                Object::Blob(_) => {}
                Object::Revision(r) => {
                    pending.push(r.root);
                    pending.extend(&r.parents);
                }
                Object::Map(Node::Leaf(bindings)) => {
                    for (_, value) in bindings {
                        entry(value, &mut pending);
                    }
                }
                Object::Map(Node::Branch {
                    terminal, children, ..
                }) => {
                    if let Some(value) = terminal {
                        entry(value, &mut pending);
                    }
                    pending.extend(children.values());
                }
                Object::Conflict(c) => {
                    for (value, _) in c.positive.iter().chain(&c.negative) {
                        entry(value, &mut pending);
                    }
                }
            }
            records.insert(id, object.encode().unwrap());
        }
        let mut out = b"AVCHIS01".to_vec();
        out.extend(self.root(head).to_bytes().unwrap());
        out.push(1);
        out.extend(head.to_bytes().unwrap());
        out.extend((records.len() as u64).to_le_bytes());
        for (id, payload) in records {
            out.extend(id.to_bytes().unwrap());
            out.extend((payload.len() as u64).to_le_bytes());
            out.extend(payload);
        }
        out.extend(b"AVCHEND1");
        out.extend(blake3::hash(&out).as_bytes());
        out
    }
    fn open(&mut self, head: Id, others: &[Id]) -> (tempfile::TempDir, Repository) {
        let mut parents = vec![head];
        parents.extend(others.iter().copied().filter(|id| *id != head));
        let carrier = self.revision(self.root(head), &parents, 250);
        let dir = tempfile::tempdir().unwrap();
        let mut repo = Repository::init(dir.path()).unwrap();
        repo.import_history(self.bundle(carrier).as_slice(), "fixture")
            .unwrap();
        repo.select_revision(head, 1, "select-fixture").unwrap();
        (dir, repo)
    }
}

struct Stack {
    dir: tempfile::TempDir,
    repo: Repository,
    base: Id,
    onto: Id,
    old: Vec<Id>,
}
fn stack() -> Stack {
    let mut g = Graph::default();
    let root = g.source(&[("base", b"base\n")]);
    let base = g.revision(root, &[], 1);
    let upstream = g.source(&[("base", b"base\n"), ("upstream", b"upstream\n")]);
    let onto = g.revision(upstream, &[base], 2);
    let first = g.source(&[("base", b"base\n"), ("feature", b"first\n")]);
    let one = g.revision(first, &[base], 3);
    let two = g.revision(first, &[one], 4); // An authored empty middle change.
    let last = g.source(&[
        ("base", b"base\n"),
        ("feature", b"last\n"),
        ("third", b"third\n"),
    ]);
    let three = g.revision(last, &[two], 5);
    let (dir, repo) = g.open(three, &[onto]);
    Stack {
        dir,
        repo,
        base,
        onto,
        old: vec![one, two, three],
    }
}
fn rewrites(receipt: &SelectionReceipt, base: Id, onto: Id) -> Vec<RevisionRewrite> {
    let Operation::Restack {
        base: actual_base,
        onto: actual_onto,
        rewrites,
    } = &receipt.operation
    else {
        panic!("restack receipt required: {receipt:?}")
    };
    assert_eq!((*actual_base, *actual_onto), (base, onto));
    rewrites.to_vec()
}
fn rows(path: &Path) -> BTreeMap<Vec<u8>, Vec<u8>> {
    rusqlite::Connection::open(path.join(".avc/store.sqlite"))
        .unwrap()
        .prepare("SELECT id, payload FROM objects ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}
fn damage(path: &Path, id: Id, remove: bool) {
    let db = rusqlite::Connection::open(path.join(".avc/store.sqlite")).unwrap();
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
fn refuse(repo: &mut Repository, path: &Path, base: Id, onto: Id, generation: u64, key: &str) {
    let before = repo.status().unwrap();
    let retained = rows(path);
    assert!(
        repo.restack(base, onto, generation, key).is_err(),
        "accepted {key}"
    );
    assert_eq!(repo.status().unwrap(), before);
    assert_eq!(rows(path), retained, "refusal admitted objects: {key}");
    assert!(repo.receipt(key).unwrap().is_none());
}
fn files(repo: &Repository, root: Id) -> BTreeMap<Vec<u8>, Vec<u8>> {
    let engine = repo.load_root(root).unwrap();
    engine
        .list(root, &[])
        .unwrap()
        .into_iter()
        .map(|(name, entry)| {
            let Entry::File { blob, .. } = entry else {
                panic!("file fixture")
            };
            let Object::Blob(bytes) = engine.object(blob).unwrap() else {
                panic!("blob fixture")
            };
            (name, bytes.clone())
        })
        .collect()
}

#[test]
fn ordered_full_mapping_preserves_metadata_empty_middle_and_each_intermediate_source() {
    let mut s = stack();
    let before = s.repo.status().unwrap();
    let retained = rows(s.dir.path());
    let originals = s
        .old
        .iter()
        .map(|id| s.repo.revision(*id).unwrap())
        .collect::<Vec<_>>();
    let live = s.dir.path().join("live");
    std::fs::write(&live, b"not selected").unwrap();
    let name: RefName = "feature/review".parse().unwrap();
    let named = s.repo.set_reference(&name, s.old[2], 0, "named").unwrap();
    // Independent ref changes cannot stale the Source generation.
    assert_eq!(s.repo.status().unwrap(), before);
    let receipt = s
        .repo
        .restack(s.base, s.onto, before.generation, "restack")
        .unwrap();
    assert_eq!(receipt.before, before);
    assert_eq!(receipt.after.generation, before.generation + 1);
    let mapping = rewrites(&receipt, s.base, s.onto);
    assert_eq!(mapping.iter().map(|r| r.before).collect::<Vec<_>>(), s.old);
    assert_eq!(mapping.len(), 3);
    assert_eq!(receipt.after.revision, Some(mapping[2].after));
    let mut parent = s.onto;
    for (index, pair) in mapping.iter().enumerate() {
        // RevisionRewrite's public contract includes Copy and Eq.
        let copied: RevisionRewrite = *pair;
        assert_eq!(copied, *pair);
        let revised = s.repo.revision(pair.after).unwrap();
        let mut expected = originals[index].clone();
        expected.parents = vec![parent];
        expected.root = revised.root;
        assert_eq!(revised, expected);
        assert_eq!(s.repo.revision(pair.before).unwrap(), originals[index]);
        assert_eq!(Object::Revision(revised.clone()).id().unwrap(), pair.after);
        let feature = if index < 2 {
            b"first\n".as_slice()
        } else {
            b"last\n"
        };
        let mut expected_files = BTreeMap::from([
            (b"base".to_vec(), b"base\n".to_vec()),
            (b"upstream".to_vec(), b"upstream\n".to_vec()),
            (b"feature".to_vec(), feature.to_vec()),
        ]);
        if index == 2 {
            expected_files.insert(b"third".to_vec(), b"third\n".to_vec());
        }
        assert_eq!(files(&s.repo, revised.root), expected_files);
        parent = pair.after;
    }
    assert_eq!(
        s.repo.revision(mapping[0].after).unwrap().root,
        s.repo.revision(mapping[1].after).unwrap().root
    );
    assert_ne!(
        mapping[0].after, mapping[1].after,
        "empty change was dropped"
    );
    for (id, payload) in retained {
        assert_eq!(rows(s.dir.path()).get(&id), Some(&payload));
    }
    assert_eq!(s.repo.reference(&name).unwrap(), Some(named.after));
    let undo = s
        .repo
        .undo("restack", receipt.after.generation, "undo")
        .unwrap();
    assert_eq!(
        (undo.after.root, undo.after.revision),
        (before.root, before.revision)
    );
    assert_eq!(undo.after.generation, receipt.after.generation + 1);
    assert_eq!(s.repo.reference(&name).unwrap(), Some(named.after));
    assert_eq!(std::fs::read(live).unwrap(), b"not selected");
    // Exported selected history owns every rewritten revision, not only its tip.
    s.repo.undo("undo", undo.after.generation, "redo").unwrap();
    let mut bytes = Vec::new();
    s.repo.export_history(&mut bytes).unwrap();
    let imported = tempfile::tempdir().unwrap();
    let mut restored = Repository::init(imported.path()).unwrap();
    restored
        .import_history(bytes.as_slice(), "history")
        .unwrap();
    for pair in mapping {
        assert_eq!(
            restored.revision(pair.after).unwrap(),
            s.repo.revision(pair.after).unwrap()
        );
    }
}

#[test]
fn one_child_is_byte_identical_to_single_rebase_and_exact_base_noop_maps_every_old_id() {
    let mut s = stack();
    let selected = s.repo.select_revision(s.old[0], 2, "first").unwrap();
    let single = s
        .repo
        .rebase(s.onto, selected.after.generation, "single")
        .unwrap();
    let reset = s
        .repo
        .undo("single", single.after.generation, "reset")
        .unwrap();
    let stack = s
        .repo
        .restack(s.base, s.onto, reset.after.generation, "one-child")
        .unwrap();
    assert_eq!(stack.after.revision, single.after.revision);
    assert_eq!(stack.after.root, single.after.root);
    assert_eq!(
        rewrites(&stack, s.base, s.onto),
        vec![RevisionRewrite {
            before: s.old[0],
            after: single.after.revision.unwrap()
        }]
    );

    let mut s = self::stack();
    let before = s.repo.status().unwrap();
    let retained = rows(s.dir.path());
    let noop = s
        .repo
        .restack(s.base, s.base, before.generation, "noop")
        .unwrap();
    assert_eq!(
        rewrites(&noop, s.base, s.base),
        s.old
            .iter()
            .map(|id| RevisionRewrite {
                before: *id,
                after: *id
            })
            .collect::<Vec<_>>()
    );
    assert_eq!(
        (noop.after.root, noop.after.revision),
        (before.root, before.revision)
    );
    assert_eq!(noop.after.generation, before.generation + 1);
    assert_eq!(rows(s.dir.path()), retained);
}

#[test]
fn empty_unrelated_wrong_kind_missing_stale_dirty_and_unborn_requests_admit_nothing() {
    let mut s = stack();
    let current = s.repo.status().unwrap();
    let missing = Id {
        kind: avc::REVISION_KIND,
        digest: [255; 32],
    };
    for (base, onto, generation, key) in [
        (s.old[2], s.onto, current.generation, "empty"),
        (s.onto, s.base, current.generation, "unrelated"),
        (current.root, s.onto, current.generation, "base-kind"),
        (s.base, current.root, current.generation, "onto-kind"),
        (missing, s.onto, current.generation, "base-missing"),
        (s.base, missing, current.generation, "onto-missing"),
        (s.base, s.onto, current.generation - 1, "stale"),
        (s.base, s.old[2], current.generation, "self"),
    ] {
        refuse(&mut s.repo, s.dir.path(), base, onto, generation, key);
    }
    let descendant = s
        .repo
        .commit("Author", "descendant", current.generation, "descendant")
        .unwrap();
    let reset = s
        .repo
        .undo("descendant", descendant.after.generation, "reset")
        .unwrap();
    refuse(
        &mut s.repo,
        s.dir.path(),
        s.base,
        descendant.after.revision.unwrap(),
        reset.after.generation,
        "descendant-target",
    );
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let dirty = s
        .repo
        .commit_source(&engine, empty, reset.after.generation, "dirty")
        .unwrap();
    refuse(
        &mut s.repo,
        s.dir.path(),
        s.base,
        s.onto,
        dirty.after.generation,
        "dirty-stack",
    );
    // Both inputs are retained and healthy even though selection is unborn.
    let mut g = Graph::default();
    let root = g.source(&[]);
    let known_base = g.revision(root, &[], 1);
    let known_onto = g.revision(root, &[known_base], 2);
    let dir = tempfile::tempdir().unwrap();
    let mut unborn = Repository::init(dir.path()).unwrap();
    unborn
        .import_history(g.bundle(known_onto).as_slice(), "fixture")
        .unwrap();
    unborn.undo("fixture", 1, "unselect").unwrap();
    refuse(&mut unborn, dir.path(), known_base, known_onto, 2, "unborn");
}

#[test]
fn merge_base_is_a_boundary_but_merges_or_parentless_stops_inside_range_are_not_linear() {
    let mut g = Graph::default();
    let root = g.source(&[("f", b"base")]);
    let left = g.revision(root, &[], 1);
    let right = g.revision(root, &[], 2);
    let base = g.revision(root, &[left, right], 3);
    let child = g.revision(root, &[base], 4);
    let tip = g.revision(root, &[child], 5);
    let onto = g.revision(root, &[right, left], 6); // Merge targets are valid too.
    let (dir, mut repo) = g.open(tip, &[onto]);
    let receipt = repo.restack(base, onto, 2, "merge-boundary").unwrap();
    assert_eq!(
        rewrites(&receipt, base, onto)
            .iter()
            .map(|p| p.before)
            .collect::<Vec<_>>(),
        vec![child, tip]
    );
    repo.undo("merge-boundary", 3, "back").unwrap();
    refuse(&mut repo, dir.path(), left, onto, 4, "included-merge");
    repo.select_revision(left, 4, "parentless-head").unwrap();
    refuse(&mut repo, dir.path(), right, onto, 5, "parentless-stop");
}

#[test]
fn change_identity_is_unique_in_range_and_disjoint_from_all_target_parents_not_globally() {
    for collision in ["range", "second-parent", "unrelated"] {
        let mut g = Graph::default();
        let root = g.source(&[("f", b"same source")]);
        let base = g.revision(root, &[], 1);
        let first = g.revision(root, &[base], 3);
        let tip = g.revision(root, &[first], if collision == "range" { 3 } else { 4 });
        let unrelated = g.revision(root, &[], 3);
        let clean = g.revision(root, &[base], 6);
        let onto = if collision == "second-parent" {
            g.revision(root, &[clean, unrelated], 7)
        } else {
            clean
        };
        let (dir, mut repo) = g.open(tip, &[onto, unrelated]);
        if collision == "unrelated" {
            assert!(
                repo.restack(base, onto, 2, "allowed").is_ok(),
                "unrelated CID reuse is not a global uniqueness violation"
            );
        } else {
            refuse(&mut repo, dir.path(), base, onto, 2, collision);
            if collision == "range" {
                refuse(&mut repo, dir.path(), base, base, 2, "invalid-noop");
            }
        }
    }
}

#[test]
fn genuine_binary_conflicts_are_retained_as_typed_source_in_rewritten_history() {
    let mut g = Graph::default();
    let base_root = g.source(&[("f", b"\0base")]);
    let base = g.revision(base_root, &[], 1);
    let onto_root = g.source(&[("f", b"\0onto")]);
    let onto = g.revision(onto_root, &[base], 2);
    let old_root = g.source(&[("f", b"\0feature")]);
    let first = g.revision(old_root, &[base], 3);
    let tip = g.revision(old_root, &[first], 4);
    let (_dir, mut repo) = g.open(tip, &[onto]);
    let receipt = repo.restack(base, onto, 2, "conflict").unwrap();
    let mapping = rewrites(&receipt, base, onto);
    let first_root = repo.revision(mapping[0].after).unwrap().root;
    assert_eq!(
        receipt.after.root, first_root,
        "empty successor must retain the conflict"
    );
    let engine = repo.load_root(first_root).unwrap();
    let Entry::Conflict(id) = engine.read(first_root, &[b"f".to_vec()]).unwrap() else {
        panic!("typed conflict expected")
    };
    let Object::Conflict(actual) = engine.object(id).unwrap() else {
        panic!("conflict object expected")
    };
    let mut positive = vec![
        (g.engine.read(onto_root, &[b"f".to_vec()]).unwrap(), 1),
        (g.engine.read(old_root, &[b"f".to_vec()]).unwrap(), 1),
    ];
    positive.sort_by_key(|(entry, _)| entry.encode().unwrap());
    assert_eq!(
        actual,
        &Conflict {
            region: 0,
            positive,
            negative: vec![(g.engine.read(base_root, &[b"f".to_vec()]).unwrap(), 1)]
        }
    );
}

#[test]
fn late_unresolved_original_comparison_base_aborts_the_whole_stack() {
    let mut g = Graph::default();
    let empty = g.source(&[]);
    let left = g.source(&[("f", b"\0left")]);
    let right = g.source(&[("f", b"\0right")]);
    let mut positive = vec![(Entry::Directory(left), 1), (Entry::Directory(right), 1)];
    positive.sort_by_key(|(entry, _)| entry.encode().unwrap());
    let object = Object::Conflict(Conflict {
        region: 1,
        positive,
        negative: vec![(Entry::Directory(empty), 1)],
    });
    let conflict = object.id().unwrap();
    g.revisions.insert(conflict, object);
    let onto = g.revision(right, &[], 1);
    let base = g.revision(conflict, &[onto], 2);
    let first = g.revision(conflict, &[base], 3);
    let tip = g.revision(left, &[first], 4);
    let (_probe_dir, mut probe) = g.open(first, &[tip]);
    let prepared_first = probe.restack(base, onto, 2, "first-only").unwrap();
    assert_eq!(prepared_first.after.root, right);
    assert_ne!(prepared_first.after.revision, Some(first));
    let (dir, mut repo) = g.open(tip, &[]);
    let before = repo.status().unwrap();
    let retained = rows(dir.path());
    // Step one is an empty change, so its new source is right. Step two then
    // compares two different resolutions against ORIGINAL parent.root=conflict.
    assert!(
        matches!(repo.restack(base, onto, 2, "late"), Err(Error::Source(avc::Error::UnresolvedBase(id))) if id == conflict)
    );
    assert_eq!(repo.status().unwrap(), before);
    assert_eq!(rows(dir.path()), retained);
    assert!(repo.receipt("late").unwrap().is_none());
}

#[test]
fn corruption_or_missing_payloads_anywhere_in_both_initial_histories_refuse_admission() {
    for (target, remove) in [
        ("old-metadata", false),
        ("onto-metadata", true),
        ("old-source", true),
        ("onto-source", false),
        ("historical-source", false),
    ] {
        let mut s = stack();
        let id = match target {
            "old-metadata" => s.old[0],
            "onto-metadata" => s.onto,
            "old-source" => Object::Blob(b"first\n".to_vec()).id().unwrap(),
            "onto-source" => Object::Blob(b"upstream\n".to_vec()).id().unwrap(),
            _ => Object::Blob(b"base\n".to_vec()).id().unwrap(),
        };
        damage(s.dir.path(), id, remove);
        refuse(&mut s.repo, s.dir.path(), s.base, s.onto, 2, "damaged");
    }
}

#[test]
fn acknowledged_full_mapping_retries_before_movement_health_and_global_key_identity_checks() {
    let mut s = stack();
    let receipt = s.repo.restack(s.base, s.onto, 2, "restack").unwrap();
    let mapping = rewrites(&receipt, s.base, s.onto);
    let name: RefName = "later".parse().unwrap();
    s.repo.set_reference(&name, s.base, 0, "ref").unwrap();
    let moved_ref = s.repo.set_reference(&name, s.onto, 1, "move-ref").unwrap();
    let mut engine = Engine::new();
    let dirty_root = engine.empty().unwrap();
    s.repo
        .commit_source(&engine, dirty_root, 3, "later-source")
        .unwrap();
    let later = s.repo.status().unwrap();
    for id in [
        mapping[0].before,
        mapping[2].before,
        mapping[0].after,
        mapping[2].after,
    ] {
        damage(s.dir.path(), id, false);
    }
    let retained = rows(s.dir.path());
    for (base, onto, generation) in [
        (s.onto, s.onto, 2),
        (s.base, s.base, 2),
        (s.base, s.onto, 3),
        (dirty_root, s.onto, 2),
    ] {
        assert!(matches!(
            s.repo.restack(base, onto, generation, "restack"),
            Err(Error::RequestKeyReuse(_))
        ));
    }
    assert!(matches!(
        s.repo.rebase(s.onto, 2, "restack"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert!(matches!(
        s.repo.commit("Author", "different kind", 2, "restack"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert!(matches!(
        s.repo.set_reference(&name, s.base, 2, "restack"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert!(matches!(
        s.repo.restack(s.base, s.onto, 2, "ref"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert!(matches!(
        s.repo.restack(s.base, s.onto, 2, "later-source"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert_eq!(
        s.repo.receipt("restack").unwrap(),
        Some(Receipt::Selection(receipt.clone()))
    );
    assert_eq!(
        s.repo.restack(s.base, s.onto, 2, "restack").unwrap(),
        receipt
    );
    drop(s.repo);
    let mut repo = Repository::open(s.dir.path()).unwrap();
    assert_eq!(
        repo.receipt("restack").unwrap(),
        Some(Receipt::Selection(receipt.clone()))
    );
    assert_eq!(repo.restack(s.base, s.onto, 2, "restack").unwrap(), receipt);
    assert_eq!(repo.status().unwrap(), later);
    assert_eq!(repo.reference(&name).unwrap(), Some(moved_ref.after));
    assert_eq!(rows(s.dir.path()), retained);
}

fn git(path: &Path, args: &[&str], input: &[u8]) -> String {
    let mut child = Command::new("git")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .current_dir(path)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn imported_anchor_is_a_valid_excluded_base_and_target_without_rewriting_foreign_history() {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(dir.path()).unwrap();
    let foreign = tempfile::tempdir().unwrap();
    git(foreign.path(), &["init", "--bare"], b"");
    let tree = git(foreign.path(), &["mktree"], b"");
    // A deliberately absent Git parent is an opaque foreign frontier, not a
    // request for restack to expand history or perform any Git I/O.
    let raw = format!(
        "tree {tree}\nparent {}\nauthor Foreign <f@example.test> 1700000000 +0000\ncommitter Foreign <f@example.test> 1700000000 +0000\n\nforeign\n",
        "a".repeat(40)
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
    let origin = repo.git_origin(anchor).unwrap();
    let anchor_revision = repo.revision(anchor).unwrap();
    assert!(anchor_revision.parents.is_empty());
    let first = repo
        .commit("Native", "first", 1, "first")
        .unwrap()
        .after
        .revision
        .unwrap();
    let second = repo
        .commit("Native", "second", 2, "second")
        .unwrap()
        .after
        .revision
        .unwrap();
    repo.select_revision(anchor, 3, "branch").unwrap();
    let onto = repo
        .commit("Native", "onto", 4, "onto")
        .unwrap()
        .after
        .revision
        .unwrap();
    repo.select_revision(second, 5, "return").unwrap();
    let retained = rows(dir.path());
    // Removing our disposable Git fixture ensures replay depends on native
    // objects only, even though the original foreign parent was never imported.
    foreign.close().unwrap();
    let receipt = repo.restack(anchor, onto, 6, "restack").unwrap();
    assert_eq!(
        rewrites(&receipt, anchor, onto)
            .iter()
            .map(|r| r.before)
            .collect::<Vec<_>>(),
        vec![first, second]
    );
    assert_eq!(repo.revision(anchor).unwrap(), anchor_revision);
    assert_eq!(repo.git_origin(anchor).unwrap(), origin);
    let back = repo.undo("restack", 7, "back").unwrap();
    let noop = repo
        .restack(anchor, anchor, back.after.generation, "anchor-target")
        .unwrap();
    assert_eq!(
        rewrites(&noop, anchor, anchor),
        vec![
            RevisionRewrite {
                before: first,
                after: first
            },
            RevisionRewrite {
                before: second,
                after: second
            }
        ]
    );
    let onto_anchor = repo
        .restack(first, anchor, noop.after.generation, "onto-anchor")
        .unwrap();
    let mapping = rewrites(&onto_anchor, first, anchor);
    assert_eq!(mapping.len(), 1);
    assert_eq!(mapping[0].before, second);
    assert_ne!(mapping[0].after, second);
    assert_eq!(
        repo.revision(mapping[0].after).unwrap().parents,
        vec![anchor]
    );
    assert_eq!(repo.git_origin(anchor).unwrap(), origin);
    let after = rows(dir.path());
    for (id, bytes) in retained {
        assert_eq!(after.get(&id), Some(&bytes));
    }
}

#[test]
fn concurrent_same_key_recovers_one_full_mapping_and_distinct_keys_have_one_cas_winner() {
    for identical in [true, false] {
        let s = stack();
        let before = s.repo.status().unwrap();
        let retained = rows(s.dir.path());
        drop(s.repo);
        let gate = Arc::new(Barrier::new(2));
        let threads = (0..2)
            .map(|index| {
                let mut repo = Repository::open(s.dir.path()).unwrap();
                let gate = gate.clone();
                let base = s.base;
                let onto = if identical || index == 0 {
                    s.onto
                } else {
                    s.base
                };
                std::thread::spawn(move || {
                    gate.wait();
                    repo.restack(
                        base,
                        onto,
                        before.generation,
                        if identical || index == 0 {
                            "race"
                        } else {
                            "other"
                        },
                    )
                })
            })
            .collect::<Vec<_>>();
        let results = threads
            .into_iter()
            .map(|t| t.join().unwrap())
            .collect::<Vec<_>>();
        if identical {
            assert_eq!(results[0].as_ref().unwrap(), results[1].as_ref().unwrap());
        } else {
            assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
            assert!(
                results
                    .iter()
                    .any(|r| matches!(r, Err(Error::Stale { .. })))
            );
        }
        let repo = Repository::open(s.dir.path()).unwrap();
        let winner = results.iter().find_map(|r| r.as_ref().ok()).unwrap();
        let Operation::Restack { base, onto, .. } = winner.operation else {
            panic!("restack winner")
        };
        assert_eq!(rewrites(winner, base, onto).len(), 3);
        assert_eq!(repo.status().unwrap(), winner.after);
        assert_eq!(winner.after.generation, before.generation + 1);
        let selected = repo.load_root(winner.after.revision.unwrap()).unwrap();
        let closure: BTreeSet<_> = selected
            .objects()
            .map(|(id, _)| id.to_bytes().unwrap().to_vec())
            .collect();
        let admitted = rows(s.dir.path());
        assert!(
            admitted
                .keys()
                .filter(|id| !retained.contains_key(*id))
                .all(|id| closure.contains(id)),
            "loser's preparation leaked objects"
        );
        assert_eq!(
            repo.receipt(&winner.request_key).unwrap(),
            Some(Receipt::Selection(winner.clone()))
        );
        if !identical {
            let loser_key = if winner.request_key == "race" {
                "other"
            } else {
                "race"
            };
            assert!(repo.receipt(loser_key).unwrap().is_none());
        }
    }
}
