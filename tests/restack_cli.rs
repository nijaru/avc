//! Exercise the real binary: parsing, acknowledgement output and no checkout effects.
use avc::{
    Entry, Id, Object,
    engine::{Edit, Engine},
    repository::{Operation, Receipt, Repository},
};
use std::{
    path::Path,
    process::{Command, Output},
};

fn cli(path: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_avc"))
        .env_remove("AVC_AUTHOR")
        .arg("--repository")
        .arg(path)
        .args(args)
        .output()
        .unwrap()
}
fn success(path: &Path, args: &[&str]) -> Output {
    let output = cli(path, args);
    assert!(output.status.success(), "{args:?}: {output:?}");
    output
}
fn source(repo: &mut Repository, files: &[(&str, &[u8])], key: &str) {
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let edits = files
        .iter()
        .map(|(name, bytes)| Edit {
            path: vec![name.as_bytes().to_vec()],
            expected: Entry::Absent,
            value: Entry::File {
                blob: engine.blob(bytes.to_vec()).unwrap(),
                executable: false,
            },
        })
        .collect::<Vec<_>>();
    let root = engine.edit(empty, &edits).unwrap();
    repo.commit_source(&engine, root, repo.status().unwrap().generation, key)
        .unwrap();
}
fn author(repo: &mut Repository, key: &str) -> Id {
    repo.commit(
        "CLI fixture Author",
        key,
        repo.status().unwrap().generation,
        key,
    )
    .unwrap()
    .after
    .revision
    .unwrap()
}
fn fixture() -> (tempfile::TempDir, Repository, Id, Id, Vec<Id>) {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(dir.path()).unwrap();
    source(&mut repo, &[("base", b"base")], "base-source");
    let base = author(&mut repo, "base");
    source(
        &mut repo,
        &[("base", b"base"), ("upstream", b"upstream")],
        "onto-source",
    );
    let onto = author(&mut repo, "onto");
    repo.select_revision(base, 4, "branch").unwrap();
    source(
        &mut repo,
        &[("base", b"base"), ("feature", b"first")],
        "first-source",
    );
    let first = author(&mut repo, "first");
    let middle = author(&mut repo, "empty-middle");
    source(
        &mut repo,
        &[("base", b"base"), ("feature", b"last")],
        "last-source",
    );
    let last = author(&mut repo, "last");
    (dir, repo, base, onto, vec![first, middle, last])
}

#[test]
fn actual_cli_requires_opt_in_and_all_identity_fields_without_authoring_inputs() {
    let (dir, repo, base, onto, _) = fixture();
    let before = repo.status().unwrap();
    let base = base.to_string();
    let onto = onto.to_string();
    let generation = before.generation.to_string();
    let identity = [
        "restack",
        "--base",
        &base,
        "--onto",
        &onto,
        "--expected-generation",
        &generation,
        "--request-key",
        "request",
    ];
    let unguarded = cli(dir.path(), &identity);
    assert!(!unguarded.status.success());
    assert!(String::from_utf8_lossy(&unguarded.stderr).contains("--experimental"));
    for removed in ["--base", "--onto", "--expected-generation", "--request-key"] {
        let mut args = vec!["--experimental"];
        let mut skip = false;
        for value in &identity {
            if skip {
                skip = false;
                continue;
            }
            if *value == removed {
                skip = true;
                continue;
            }
            args.push(*value);
        }
        let output = cli(dir.path(), &args);
        assert!(!output.status.success(), "missing {removed} accepted");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(removed),
            "{output:?}"
        );
    }
    for forbidden in ["--author", "--message", "--time-ns"] {
        let mut args = vec!["--experimental"];
        args.extend(identity);
        args.extend([forbidden, "not-part-of-restack"]);
        assert!(!cli(dir.path(), &args).status.success());
    }
    assert_eq!(repo.status().unwrap(), before);
    assert!(repo.receipt("request").unwrap().is_none());
}

#[test]
fn actual_cli_prints_every_ordered_pair_recovers_receipt_and_undoes_without_checkout() {
    let (dir, mut repo, base, onto, old) = fixture();
    let before = repo.status().unwrap();
    let live = dir.path().join("live");
    std::fs::write(&live, b"live file must not change").unwrap();
    let base_arg = base.to_string();
    let onto_arg = onto.to_string();
    let generation = before.generation.to_string();
    let args = [
        "--experimental",
        "restack",
        "--base",
        &base_arg,
        "--onto",
        &onto_arg,
        "--expected-generation",
        &generation,
        "--request-key",
        "restack",
    ];
    let output = success(dir.path(), &args);
    let Receipt::Selection(receipt) = repo.receipt("restack").unwrap().unwrap() else {
        panic!("selection receipt")
    };
    let Operation::Restack {
        base: got_base,
        onto: got_onto,
        rewrites,
    } = &receipt.operation
    else {
        panic!("restack operation")
    };
    assert_eq!((*got_base, *got_onto), (base, onto));
    assert_eq!(rewrites.iter().map(|r| r.before).collect::<Vec<_>>(), old);
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    let header = stdout.lines().next().unwrap();
    for field in [
        format!("base={base}"),
        format!("onto={onto}"),
        "rewrites=3".to_owned(),
        format!("generation={}", before.generation + 1),
        "request=restack".to_owned(),
    ] {
        assert!(header.contains(&field), "missing {field} in {stdout}");
    }
    assert_eq!(
        stdout.lines().skip(1).collect::<Vec<_>>(),
        rewrites
            .iter()
            .map(|r| format!("Rewrite: {} -> {}", r.before, r.after))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        success(dir.path(), &["receipt", "restack"]).stdout,
        output.stdout
    );
    success(
        dir.path(),
        &[
            "verify",
            "--root",
            &receipt.after.revision.unwrap().to_string(),
        ],
    );
    success(
        dir.path(),
        &[
            "--experimental",
            "history",
            "undo",
            "restack",
            "--expected-generation",
            &receipt.after.generation.to_string(),
            "--request-key",
            "undo",
        ],
    );
    let undone = repo.status().unwrap();
    assert_eq!(
        (undone.root, undone.revision),
        (before.root, before.revision)
    );
    assert_eq!(undone.generation, receipt.after.generation + 1);
    assert_eq!(success(dir.path(), &args).stdout, output.stdout);
    assert_eq!(repo.status().unwrap(), undone);
    assert_eq!(std::fs::read(live).unwrap(), b"live file must not change");
    source(&mut repo, &[("dirty", b"later selected source")], "dirty");
    let later = repo.status().unwrap();
    let refused = cli(
        dir.path(),
        &[
            "--experimental",
            "restack",
            "--base",
            &base_arg,
            "--onto",
            &onto_arg,
            "--expected-generation",
            &later.generation.to_string(),
            "--request-key",
            "dirty-restack",
        ],
    );
    assert!(!refused.status.success());
    assert_eq!(repo.status().unwrap(), later);
    assert!(repo.receipt("dirty-restack").unwrap().is_none());
    // Recovered immutable outcomes cannot depend on old or new revision bytes.
    let db = rusqlite::Connection::open(dir.path().join(".avc/store.sqlite")).unwrap();
    db.execute_batch("DROP TRIGGER objects_no_update").unwrap();
    for id in [old[0], rewrites.last().unwrap().after] {
        assert_eq!(
            db.execute(
                "UPDATE objects SET payload = zeroblob(length) WHERE id = ?1",
                [id.to_bytes().unwrap().as_slice()]
            )
            .unwrap(),
            1
        );
    }
    assert_eq!(success(dir.path(), &args).stdout, output.stdout);
    assert_eq!(
        success(dir.path(), &["receipt", "restack"]).stdout,
        output.stdout
    );
    assert_eq!(repo.status().unwrap(), later);
}

#[test]
fn base_and_onto_are_exact_runtime_native_ids_and_rejected_values_cannot_inject_controls() {
    let (dir, repo, base, onto, _) = fixture();
    let before = repo.status().unwrap();
    let generation = before.generation.to_string();
    let base = base.to_string();
    let onto = onto.to_string();
    let wrong_kind = Object::Blob(b"not a revision".to_vec())
        .id()
        .unwrap()
        .to_string();
    for field in ["--base", "--onto"] {
        let mut errors = Vec::new();
        for invalid in [
            "feature/ref",
            "avc1-k05-abcd",
            "\x1b[31mforged\nreceipt\r\t\x07",
            &wrong_kind,
        ] {
            let mut args = vec![
                "--experimental",
                "restack",
                "--base",
                &base,
                "--onto",
                &onto,
                "--expected-generation",
                &generation,
                "--request-key",
                "invalid",
            ];
            let index = args.iter().position(|v| *v == field).unwrap() + 1;
            args[index] = invalid;
            let output = cli(dir.path(), &args);
            assert!(!output.status.success(), "accepted {invalid:?}");
            assert!(output.stdout.is_empty());
            assert!(
                !output
                    .stderr
                    .iter()
                    .any(|b| matches!(b, 0x1b | 0x07 | b'\r' | b'\t'))
            );
            assert!(!String::from_utf8_lossy(&output.stderr).contains("forged"));
            if invalid != wrong_kind {
                errors.push(output.stderr);
            } else {
                assert!(!String::from_utf8_lossy(&output.stderr).contains(&wrong_kind));
            }
        }
        assert!(
            errors.windows(2).all(|pair| pair[0] == pair[1]),
            "malformed ID validation must use constant runtime errors, not echoed parser inputs: {errors:?}"
        );
    }
    assert_eq!(repo.status().unwrap(), before);
    assert!(repo.receipt("invalid").unwrap().is_none());
}
