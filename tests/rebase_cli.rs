use avc::{
    Entry, Id, Object,
    engine::{Edit, Engine},
    repository::Repository,
};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn cli(project: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_avc"))
        .env_remove("AVC_AUTHOR")
        .arg("--repository")
        .arg(project)
        .args(args)
        .output()
        .unwrap()
}
fn success(project: &Path, args: &[&str]) -> Output {
    let output = cli(project, args);
    assert!(output.status.success(), "{args:?}: {output:?}");
    output
}
fn select(repo: &mut Repository, value: &[u8], key: &str) {
    let mut engine = Engine::new();
    let empty = engine.empty().unwrap();
    let blob = engine.blob(value.to_vec()).unwrap();
    let root = engine
        .edit(
            empty,
            &[Edit {
                path: vec![b"f".to_vec()],
                expected: Entry::Absent,
                value: Entry::File {
                    blob,
                    executable: false,
                },
            }],
        )
        .unwrap();
    repo.commit_source(&engine, root, repo.status().unwrap().generation, key)
        .unwrap();
}
fn commit(repo: &mut Repository, key: &str) -> Id {
    repo.commit("Author", key, repo.status().unwrap().generation, key)
        .unwrap()
        .after
        .revision
        .unwrap()
}

#[test]
fn actual_cli_reports_empty_or_unresolved_replay_recovers_receipts_and_exports_checked_history() {
    for empty in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let live = p.join("live");
        fs::write(&live, b"never selected or overwritten").unwrap();
        let mut repo = Repository::init(p).unwrap();
        select(&mut repo, b"\0base", "base-source");
        let base = commit(&mut repo, "base");
        select(&mut repo, b"\0onto", "onto-source");
        let onto = commit(&mut repo, "onto");
        repo.undo("onto", 4, "back").unwrap();
        select(
            &mut repo,
            if empty { b"\0onto" } else { b"\0tip" },
            "tip-source",
        );
        let tip = commit(&mut repo, "tip");
        let original = repo.revision(tip).unwrap();
        let onto_arg = onto.to_string();
        let unguarded = cli(
            p,
            &[
                "rebase",
                "--onto",
                &onto_arg,
                "--expected-generation",
                "7",
                "--request-key",
                "rebase",
            ],
        );
        assert!(!unguarded.status.success());
        assert_eq!(repo.status().unwrap().generation, 7);
        let args = [
            "--experimental",
            "rebase",
            "--onto",
            &onto_arg,
            "--expected-generation",
            "7",
            "--request-key",
            "rebase",
        ];
        let output = success(p, &args);
        let diagnostic = String::from_utf8(output.stderr).unwrap();
        if empty {
            assert!(diagnostic.contains("Empty change"));
            assert!(!diagnostic.contains("Unresolved source conflicts"));
        } else {
            assert!(diagnostic.contains("Unresolved source conflicts preserved"));
            assert!(!diagnostic.contains("Empty change"));
        }
        let receipt = repo.receipt("rebase").unwrap().unwrap();
        let revised = repo.revision(receipt.after.revision.unwrap()).unwrap();
        assert_eq!(revised.parents, vec![onto]);
        assert_eq!(revised.change, original.change);
        assert_eq!(
            (revised.author, revised.message, revised.time_ns),
            (original.author, original.message, original.time_ns)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains(&format!("onto={onto}")));
        success(
            p,
            &[
                "verify",
                "--root",
                &receipt.after.revision.unwrap().to_string(),
            ],
        );
        select(&mut repo, b"later dirty selected bytes", "later");
        let later = repo.status().unwrap();
        assert_eq!(success(p, &args).stdout, output.stdout);
        let changed = cli(
            p,
            &[
                "--experimental",
                "rebase",
                "--onto",
                &base.to_string(),
                "--expected-generation",
                "7",
                "--request-key",
                "rebase",
            ],
        );
        assert!(!changed.status.success());
        let dirty = cli(
            p,
            &[
                "--experimental",
                "rebase",
                "--onto",
                &onto_arg,
                "--expected-generation",
                "9",
                "--request-key",
                "new-rebase",
            ],
        );
        assert!(!dirty.status.success());
        assert!(String::from_utf8_lossy(&dirty.stderr).contains("selected-but-uncommitted"));
        assert_eq!(repo.status().unwrap(), later);
        assert_eq!(fs::read(live).unwrap(), b"never selected or overwritten");
        let bundle = p.join("history.avc");
        success(
            p,
            &["export", "--history", "--output", bundle.to_str().unwrap()],
        );
        if std::env::var_os("AVC_REFERENCE_CHECK").is_some() {
            let checked = Command::new("uv")
                .args(["run", "reference/bundle.py", "--history", "--json"])
                .arg(&bundle)
                .output()
                .unwrap();
            assert!(
                checked.status.success(),
                "independent rebase history: {checked:?}"
            );
            let corpus: serde_json::Value = serde_json::from_slice(&checked.stdout).unwrap();
            assert_eq!(
                corpus["roots"],
                serde_json::json!([later.root.to_string(), later.revision.unwrap().to_string()])
            );
        }
        if empty {
            // A previously acknowledged rewrite may outlive later damage to its
            // source. Recover the receipt, warn on optional diagnostics, never
            // turn a committed outcome into a false failed mutation.
            let blob = Object::Blob(b"\0onto".to_vec()).id().unwrap();
            let db = rusqlite::Connection::open(p.join(".avc/store.sqlite")).unwrap();
            db.execute_batch("DROP TRIGGER objects_no_update").unwrap();
            db.execute(
                "UPDATE objects SET payload = ?1, length = ?2 WHERE id = ?3",
                rusqlite::params![b"broken".as_slice(), 6, blob.to_bytes().unwrap().as_slice()],
            )
            .unwrap();
            let retry = success(p, &args);
            assert_eq!(retry.stdout, output.stdout);
            assert!(
                String::from_utf8_lossy(&retry.stderr)
                    .contains("Rebase committed; comparison diagnostics unavailable")
            );
            assert_eq!(repo.status().unwrap(), later);
            assert!(repo.load_root(receipt.after.revision.unwrap()).is_err());
        }
    }
}
