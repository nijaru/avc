use avc::{
    Entry, Id,
    engine::{Edit, Engine},
    repository::{
        Error, NamedRef, Receipt, RefName, RefReceipt, RefValue, Repository, SelectionReceipt,
    },
};
use rusqlite::Connection;
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, Barrier},
};

fn name(value: &str) -> RefName {
    value.parse().unwrap()
}

fn commit(repo: &mut Repository, key: &str) -> Id {
    repo.commit("Author", key, repo.status().unwrap().generation, key)
        .unwrap()
        .after
        .revision
        .unwrap()
}

fn select(repo: &mut Repository, bytes: &[u8], key: &str) -> Id {
    let mut engine = Engine::new();
    let root = engine.empty().unwrap();
    let blob = engine.blob(bytes.to_vec()).unwrap();
    let root = engine
        .edit(
            root,
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
    let _: SelectionReceipt = repo
        .commit_source(&engine, root, repo.status().unwrap().generation, key)
        .unwrap();
    blob
}

fn database(project: &Path) -> Connection {
    Connection::open(project.join(".avc/store.sqlite")).unwrap()
}

// SQLite owns every database descriptor, including during damage injection.
fn retained_objects(project: &Path) -> Vec<(Vec<u8>, i64, Vec<u8>)> {
    let db = database(project);
    db.prepare("SELECT id, length, payload FROM objects ORDER BY id")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn corrupt(project: &Path, id: Id) {
    let db = database(project);
    db.execute_batch("DROP TRIGGER IF EXISTS objects_no_update")
        .unwrap();
    assert_eq!(
        db.execute(
            "UPDATE objects SET payload = zeroblob(length) WHERE id = ?1",
            [id.to_bytes().unwrap().as_slice()],
        )
        .unwrap(),
        1
    );
}

fn remove(project: &Path, id: Id) {
    let db = database(project);
    db.pragma_update(None, "foreign_keys", false).unwrap();
    db.execute_batch("DROP TRIGGER objects_no_delete").unwrap();
    assert_eq!(
        db.execute(
            "DELETE FROM objects WHERE id = ?1",
            [id.to_bytes().unwrap().as_slice()]
        )
        .unwrap(),
        1
    );
}

#[test]
fn names_are_exact_utf8_flat_keys_not_git_paths() {
    for valid in [
        "a",
        "a/b",
        "A",
        "é",
        "e\u{301}",
        ".git",
        "topic.lock",
        "a/.git/b.lock",
        "a\\b",
        "a\"b",
        "a:b",
        "@{x}",
        "topic/\u{202e}safe",
    ] {
        assert_eq!(name(valid).as_str(), valid);
    }
    for invalid in [
        "",
        "/a",
        "a/",
        "a//b",
        ".",
        "..",
        "a/./b",
        "a/../b",
        " ",
        "a b",
        "a\tb",
        "a\nb",
        "a\rb",
        "a\0b",
        "a\u{1b}b",
        "a\u{7f}b",
        "a\u{85}b",
        "a\u{a0}b",
        "a\u{2003}b",
    ] {
        assert!(invalid.parse::<RefName>().is_err(), "accepted {invalid:?}");
    }
    assert!("x".repeat(1024).parse::<RefName>().is_ok());
    assert!("x".repeat(1025).parse::<RefName>().is_err());
    assert!("é".repeat(512).parse::<RefName>().is_ok());
    assert!(format!("{}x", "é".repeat(512)).parse::<RefName>().is_err());
}

#[test]
fn setting_and_advancing_refs_changes_neither_selection_nor_retained_payloads() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    assert!(repo.references(None, 20).unwrap().is_empty());
    assert_eq!(repo.reference(&name("main")).unwrap(), None);
    select(&mut repo, b"selected", "source");
    let revision = commit(&mut repo, "commit");
    let selected = repo.status().unwrap();
    assert!(repo.references(None, 20).unwrap().is_empty());
    let operations_before: i64 = database(project.path())
        .query_row("SELECT count(*) FROM operations", [], |r| r.get(0))
        .unwrap();
    let objects = retained_objects(project.path());
    let mut source_export = Vec::new();
    repo.export(selected.root, &mut source_export).unwrap();
    let mut history_export = Vec::new();
    repo.export_history(&mut history_export).unwrap();
    let live = project.path().join("live");
    std::fs::write(&live, b"never captured").unwrap();
    let first: RefReceipt = repo
        .set_reference(&name("release"), revision, 0, "ref-one")
        .unwrap();
    assert_eq!(first.request_key, "ref-one");
    assert_eq!(first.name, name("release"));
    assert_eq!(first.before, None);
    assert_eq!(
        first.after,
        RefValue {
            revision,
            generation: 1
        }
    );
    let second = repo
        .set_reference(&name("release"), revision, 1, "ref-two")
        .unwrap();
    assert_eq!(second.before, Some(first.after));
    assert_eq!(
        second.after,
        RefValue {
            revision,
            generation: 2
        }
    );
    assert_eq!(repo.status().unwrap(), selected);
    assert_eq!(retained_objects(project.path()), objects);
    let mut source_after = Vec::new();
    repo.export(selected.root, &mut source_after).unwrap();
    let mut history_after = Vec::new();
    repo.export_history(&mut history_after).unwrap();
    assert_eq!(source_after, source_export, "source transfer excludes refs");
    assert_eq!(
        history_after, history_export,
        "selected-history transfer excludes refs"
    );
    assert_eq!(std::fs::read(live).unwrap(), b"never captured");
    assert_eq!(
        repo.references(None, 20).unwrap(),
        vec![NamedRef {
            name: name("release"),
            value: second.after
        }]
    );
    assert_eq!(
        repo.set_reference(&name("release"), revision, 0, "ref-one")
            .unwrap(),
        first
    );
    let operations_after: i64 = database(project.path())
        .query_row("SELECT count(*) FROM operations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        operations_after - operations_before,
        2,
        "one journal row per accepted set, none for retry"
    );
    assert_eq!(repo.receipt("ref-one").unwrap(), Some(Receipt::Ref(first)));
    assert!(matches!(
        repo.receipt("commit").unwrap(),
        Some(Receipt::Selection(_))
    ));
    drop(repo);
    let repo = Repository::open(project.path()).unwrap();
    assert_eq!(
        repo.reference(&name("release")).unwrap(),
        Some(second.after)
    );
    assert_eq!(repo.status().unwrap(), selected);
}

#[test]
fn source_commit_amend_rebase_and_undo_never_advance_a_ref() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let base = commit(&mut repo, "base");
    let pinned = repo
        .set_reference(&name("main"), base, 0, "pin")
        .unwrap()
        .after;
    select(&mut repo, b"new contents", "source");
    assert_eq!(repo.reference(&name("main")).unwrap(), Some(pinned));
    commit(&mut repo, "tip");
    assert_eq!(repo.reference(&name("main")).unwrap(), Some(pinned));
    repo.amend(
        "Author",
        "amended",
        repo.status().unwrap().generation,
        "amend",
    )
    .unwrap();
    assert_eq!(repo.reference(&name("main")).unwrap(), Some(pinned));
    repo.rebase(base, repo.status().unwrap().generation, "rebase")
        .unwrap();
    assert_eq!(repo.reference(&name("main")).unwrap(), Some(pinned));
    repo.undo("rebase", repo.status().unwrap().generation, "undo")
        .unwrap();
    assert_eq!(repo.reference(&name("main")).unwrap(), Some(pinned));
    assert_eq!(repo.references(None, 20).unwrap().len(), 1);
}

#[test]
fn retries_recover_original_before_and_after_not_current_ref_or_source() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let first = commit(&mut repo, "first");
    let second = commit(&mut repo, "second");
    let n = name("topic");
    let birth = repo.set_reference(&n, first, 0, "birth").unwrap();
    let movement = repo.set_reference(&n, second, 1, "movement").unwrap();
    repo.set_reference(&n, first, 2, "later").unwrap();
    select(&mut repo, b"unrelated later source", "source");
    let selected = repo.status().unwrap();
    assert_eq!(repo.set_reference(&n, first, 0, "birth").unwrap(), birth);
    assert_eq!(
        repo.set_reference(&n, second, 1, "movement").unwrap(),
        movement
    );
    assert_eq!(
        movement.before,
        Some(RefValue {
            revision: first,
            generation: 1
        })
    );
    assert_eq!(
        repo.receipt("movement").unwrap(),
        Some(Receipt::Ref(movement))
    );
    assert_eq!(
        repo.reference(&n).unwrap(),
        Some(RefValue {
            revision: first,
            generation: 3
        })
    );
    for (other_name, target, generation) in [
        (name("other"), first, 0),
        (n.clone(), second, 0),
        (n.clone(), first, 1),
        (n.clone(), selected.root, 0),
        (n.clone(), first, u64::MAX),
    ] {
        assert!(matches!(
            repo.set_reference(&other_name, target, generation, "birth"),
            Err(Error::RequestKeyReuse(_))
        ));
    }
    assert!(repo.set_reference(&n, second, 0, "stale").is_err());
    assert!(repo.receipt("stale").unwrap().is_none());
    assert_eq!(repo.status().unwrap(), selected);
}

#[test]
fn request_keys_are_global_and_ref_receipts_are_not_source_undo_or_capture() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let revision = commit(&mut repo, "selection-key");
    let n = name("topic");
    assert!(matches!(
        repo.set_reference(&n, revision, 0, "selection-key"),
        Err(Error::RequestKeyReuse(_))
    ));
    repo.set_reference(&n, revision, 0, "ref-key").unwrap();
    let selected = repo.status().unwrap();
    assert!(matches!(
        repo.commit("Author", "message", selected.generation, "ref-key"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert!(matches!(
        repo.commit_source(
            &Engine::new(),
            selected.root,
            selected.generation,
            "ref-key"
        ),
        Err(Error::RequestKeyReuse(_))
    ));
    assert!(matches!(
        repo.preparation_base(selected.generation, "ref-key"),
        Err(Error::RequestKeyReuse(_))
    ));
    assert!(
        repo.undo("ref-key", selected.generation, "undo-ref")
            .is_err()
    );
    assert!(repo.receipt("undo-ref").unwrap().is_none());
    assert!(repo.capture_observation("ref-key").unwrap().is_none());
    assert_eq!(repo.status().unwrap(), selected);
    // The ref did not take ownership of the current selection operation.
    repo.undo("selection-key", selected.generation, "undo-selection")
        .unwrap();
    assert_eq!(
        repo.reference(&n).unwrap(),
        Some(RefValue {
            revision,
            generation: 1
        })
    );
}

#[test]
fn list_uses_binary_utf8_order_and_exclusive_nonexistent_cursors() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let revision = commit(&mut repo, "commit");
    let mut names = vec!["é", "a/b", "Z", "e\u{301}", "a", "A", "ä", ".git", "x.lock"];
    for (i, n) in names.iter().enumerate() {
        repo.set_reference(&name(n), revision, 0, &format!("ref-{i}"))
            .unwrap();
    }
    names.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    let listed = repo.references(None, 100_000).unwrap();
    assert_eq!(
        listed.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
        names
    );
    assert!(repo.references(None, 0).unwrap().is_empty());
    assert!(repo.references(None, 100_001).is_err());
    let mut paged = Vec::new();
    let mut after = None;
    loop {
        let page = repo.references(after.as_ref(), 2).unwrap();
        if page.is_empty() {
            break;
        }
        after = Some(page.last().unwrap().name.clone());
        paged.extend(page);
    }
    assert_eq!(paged, listed);
    let cursor = name("a/a");
    assert_eq!(
        repo.references(Some(&cursor), 2).unwrap(),
        listed
            .iter()
            .filter(|r| r.name.as_str().as_bytes() > cursor.as_str().as_bytes())
            .take(2)
            .cloned()
            .collect::<Vec<_>>()
    );
    assert!(repo.references(Some(&name("🦀")), 20).unwrap().is_empty());
    // Updates do not duplicate names or consume the result limit.
    for generation in 1..25 {
        repo.set_reference(
            &name("a"),
            revision,
            generation,
            &format!("advance-{generation}"),
        )
        .unwrap();
    }
    let latest = repo.references(None, names.len()).unwrap();
    assert_eq!(latest.len(), names.len());
    assert_eq!(
        latest.iter().find(|r| r.name == name("a")).unwrap().value,
        RefValue {
            revision,
            generation: 25
        }
    );
    assert_eq!(
        repo.references(Some(&name("A")), 1).unwrap()[0].name,
        name("Z")
    );
}

#[test]
fn same_name_cas_has_one_winner_and_identical_requests_share_one_receipt() {
    for identical in [false, true] {
        let project = tempfile::tempdir().unwrap();
        let mut repo = Repository::init(project.path()).unwrap();
        let revision = commit(&mut repo, "commit");
        let selected = repo.status().unwrap();
        drop(repo);
        let barrier = Arc::new(Barrier::new(2));
        let workers = (0..2)
            .map(|i| {
                let mut repo = Repository::open(project.path()).unwrap();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    repo.set_reference(
                        &name("race"),
                        revision,
                        0,
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
            assert_eq!(results.iter().filter(|r| r.is_err()).count(), 1);
        }
        let repo = Repository::open(project.path()).unwrap();
        assert_eq!(
            repo.reference(&name("race")).unwrap(),
            Some(RefValue {
                revision,
                generation: 1
            })
        );
        assert_eq!(repo.status().unwrap(), selected);
        assert_eq!(
            ["one", "two"]
                .iter()
                .filter(|k| repo.receipt(k).unwrap().is_some())
                .count(),
            1
        );
    }
}

#[test]
fn different_names_and_source_mutations_have_independent_cas_domains() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let revision = commit(&mut repo, "commit");
    let generation = repo.status().unwrap().generation;
    drop(repo);
    let barrier = Arc::new(Barrier::new(3));
    let workers = (0..3)
        .map(|i| {
            let mut repo = Repository::open(project.path()).unwrap();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                if i == 2 {
                    repo.commit("Author", "later", generation, "later")
                        .map(|_| ())
                } else {
                    repo.set_reference(
                        &name(if i == 0 { "a" } else { "b" }),
                        revision,
                        0,
                        if i == 0 { "ref-a" } else { "ref-b" },
                    )
                    .map(|_| ())
                }
            })
        })
        .collect::<Vec<_>>();
    for worker in workers {
        worker.join().unwrap().unwrap();
    }
    let repo = Repository::open(project.path()).unwrap();
    assert_eq!(repo.status().unwrap().generation, generation + 1);
    for n in ["a", "b"] {
        assert_eq!(
            repo.reference(&name(n)).unwrap(),
            Some(RefValue {
                revision,
                generation: 1
            })
        );
    }
}

#[test]
fn fresh_sets_validate_revision_kind_and_direct_dependency_presence_atomically() {
    for missing_parent in [false, true] {
        let project = tempfile::tempdir().unwrap();
        let mut repo = Repository::init(project.path()).unwrap();
        let parent = commit(&mut repo, "parent");
        select(&mut repo, b"unique child source", "source");
        let child = commit(&mut repo, "child");
        let selected = repo.status().unwrap();
        let target = if missing_parent {
            parent
        } else {
            repo.revision(child).unwrap().root
        };
        assert!(
            repo.set_reference(&name("wrong-kind"), selected.root, 0, "wrong-kind")
                .is_err()
        );
        assert!(repo.receipt("wrong-kind").unwrap().is_none());
        let unknown = Id {
            kind: avc::REVISION_KIND,
            digest: [0; 32],
        };
        assert!(
            repo.set_reference(&name("unknown"), unknown, 0, "unknown")
                .is_err()
        );
        assert!(repo.receipt("unknown").unwrap().is_none());
        remove(project.path(), target);
        assert!(
            repo.set_reference(&name("missing-dependency"), child, 0, "missing-dependency")
                .is_err()
        );
        assert_eq!(repo.reference(&name("missing-dependency")).unwrap(), None);
        assert!(repo.receipt("missing-dependency").unwrap().is_none());
        assert_eq!(repo.status().unwrap(), selected);
    }
}

#[test]
fn fresh_set_requires_dependency_presence_not_dependency_payload_health() {
    for damage_parent in [false, true] {
        let project = tempfile::tempdir().unwrap();
        let mut repo = Repository::init(project.path()).unwrap();
        let parent = commit(&mut repo, "parent");
        select(&mut repo, b"child source", "source");
        let child = commit(&mut repo, "child");
        let dependency = if damage_parent {
            parent
        } else {
            repo.revision(child).unwrap().root
        };
        corrupt(project.path(), dependency);
        assert!(repo.load_root(child).is_err());
        let receipt = repo
            .set_reference(&name("metadata-only"), child, 0, "set")
            .unwrap();
        assert_eq!(
            receipt.after,
            RefValue {
                revision: child,
                generation: 1
            }
        );
    }
}

#[test]
fn replay_and_metadata_reads_survive_target_damage_but_fresh_set_does_not() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let first = commit(&mut repo, "first");
    let second = commit(&mut repo, "second");
    let n = name("topic");
    let birth = repo.set_reference(&n, first, 0, "birth").unwrap();
    let moved = repo.set_reference(&n, second, 1, "moved").unwrap();
    corrupt(project.path(), first);
    corrupt(project.path(), second);
    drop(repo);
    let mut repo = Repository::open(project.path()).unwrap();
    assert!(repo.revision(first).is_err());
    assert!(repo.revision(second).is_err());
    assert_eq!(repo.set_reference(&n, first, 0, "birth").unwrap(), birth);
    assert_eq!(repo.set_reference(&n, second, 1, "moved").unwrap(), moved);
    assert_eq!(
        repo.receipt("moved").unwrap(),
        Some(Receipt::Ref(moved.clone()))
    );
    assert_eq!(repo.reference(&n).unwrap(), Some(moved.after));
    assert_eq!(
        repo.references(None, 20).unwrap(),
        vec![NamedRef {
            name: n.clone(),
            value: moved.after
        }]
    );
    assert!(repo.set_reference(&n, second, 2, "fresh").is_err());
    assert!(repo.receipt("fresh").unwrap().is_none());
    assert_eq!(repo.reference(&n).unwrap(), Some(moved.after));
}

#[test]
fn fresh_ref_set_does_not_verify_historical_blob_payloads() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let historical_blob = select(&mut repo, b"historical payload", "old-source");
    let parent = commit(&mut repo, "parent");
    select(&mut repo, b"current payload", "new-source");
    let tip = commit(&mut repo, "tip");
    corrupt(project.path(), historical_blob);
    assert!(repo.load_root(parent).is_err());
    let set = repo
        .set_reference(&name("metadata-only"), tip, 0, "set")
        .unwrap();
    assert_eq!(
        set.after,
        RefValue {
            revision: tip,
            generation: 1
        }
    );
    assert_eq!(
        repo.reference(&name("metadata-only")).unwrap(),
        Some(set.after)
    );
}

fn git(dir: &Path, args: &[&str], input: &[u8]) -> Vec<u8> {
    let mut child = Command::new("git")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .current_dir(dir)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "gc.auto=0",
        ])
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
    output.stdout
}

fn git_text(dir: &Path, args: &[&str], input: &[u8]) -> String {
    String::from_utf8(git(dir, args, input))
        .unwrap()
        .trim()
        .to_owned()
}

#[test]
fn git_anchors_are_valid_ref_targets_without_origin_custody_or_git_ref_side_effects() {
    let project = tempfile::tempdir().unwrap();
    let foreign = project.path().join("foreign.git");
    git(
        project.path(),
        &[
            "init",
            "--bare",
            "--object-format=sha1",
            foreign.to_str().unwrap(),
        ],
        b"",
    );
    let tree = git_text(&foreign, &["mktree"], b"");
    let raw_commit = format!(
        "tree {tree}\nauthor Fixture <fixture@example.invalid> 1700000000 +0000\ncommitter Fixture <fixture@example.invalid> 1700000000 +0000\n\nanchor\n"
    );
    let oid = git_text(
        &foreign,
        &["hash-object", "-w", "-t", "commit", "--stdin"],
        raw_commit.as_bytes(),
    );
    git(&foreign, &["update-ref", "refs/heads/main", &oid], b"");
    let mut repo = Repository::init(project.path()).unwrap();
    let anchor = repo
        .import_git(&foreign, oid.parse().unwrap(), "import")
        .unwrap()
        .after
        .revision
        .unwrap();
    let tip = repo
        .commit(
            "Author <author@example.invalid>",
            "native child",
            repo.status().unwrap().generation,
            "native-child",
        )
        .unwrap()
        .after
        .revision
        .unwrap();
    assert!(repo.references(None, 20).unwrap().is_empty());
    repo.export_git(&foreign, tip).unwrap();
    let selected = repo.status().unwrap();
    let origin = repo.git_origin(anchor).unwrap().unwrap();
    let objects = retained_objects(project.path());
    let refs = git(&foreign, &["show-ref"], b"");
    let mappings = || {
        let db = database(project.path());
        db.prepare(
            "SELECT revision, algorithm, policy, commit_oid FROM git_exports ORDER BY revision",
        )
        .unwrap()
        .query_map([], |r| {
            Ok((
                r.get::<_, Vec<u8>>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Vec<u8>>(3)?,
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>()
    };
    let exports = mappings();
    assert!(!exports.is_empty());
    repo.set_reference(&name("main"), anchor, 0, "ref-anchor")
        .unwrap();
    repo.set_reference(&name("main"), tip, 1, "ref-tip")
        .unwrap();
    assert_eq!(repo.status().unwrap(), selected);
    assert_eq!(repo.git_origin(anchor).unwrap(), Some(origin));
    assert_eq!(mappings(), exports);
    assert_eq!(retained_objects(project.path()), objects);
    assert_eq!(git(&foreign, &["show-ref"], b""), refs);
    let db = database(project.path());
    assert!(
        db.execute("DELETE FROM git_origins", []).is_err(),
        "ref writes must not remove origin custody protection"
    );
    db.pragma_update(None, "foreign_keys", false).unwrap();
    db.execute_batch("DROP TRIGGER git_origins_no_delete; DELETE FROM git_origins;")
        .unwrap();
    drop(db);
    // An alias is not an origin-health certificate, even for an imported anchor.
    let set = repo
        .set_reference(
            &name("anchor-with-damaged-custody"),
            anchor,
            0,
            "ref-missing-origin",
        )
        .unwrap();
    assert_eq!(
        set.after,
        RefValue {
            revision: anchor,
            generation: 1
        }
    );
    assert_eq!(repo.status().unwrap(), selected);
    assert_eq!(git(&foreign, &["show-ref"], b""), refs);
}

#[test]
fn generation_and_key_boundaries_reject_without_creating_refs_or_receipts() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let revision = commit(&mut repo, "commit");
    let n = name("topic");
    let selected = repo.status().unwrap();
    for expected in [1, i64::MAX as u64, i64::MAX as u64 + 1, u64::MAX] {
        assert!(
            repo.set_reference(&n, revision, expected, "bad-generation")
                .is_err()
        );
        assert!(repo.receipt("bad-generation").unwrap().is_none());
    }
    for key in [String::new(), "x".repeat(1025)] {
        assert!(repo.set_reference(&n, revision, 0, &key).is_err());
    }
    assert_eq!(repo.reference(&n).unwrap(), None);
    assert_eq!(repo.status().unwrap(), selected);
    let key = "x".repeat(1024);
    repo.set_reference(&n, revision, 0, &key).unwrap();
    assert!(repo.receipt(&key).unwrap().is_some());
}
