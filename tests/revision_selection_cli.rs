use avc::{
    Entry, Id,
    engine::{Edit, Engine},
    repository::{Operation, Receipt, Repository},
};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn command(project: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_avc"));
    command
        .arg("--repository")
        .arg(project)
        .args(args)
        .env_remove("AVC_AUTHOR");
    command
}

fn success(project: &Path, args: &[&str]) -> Output {
    let output = command(project, args).output().unwrap();
    assert!(output.status.success(), "{args:?}: {output:?}");
    output
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn fixture(project: &Path) -> (Id, Id) {
    let mut repo = Repository::init(project).unwrap();
    let mut revisions = Vec::new();
    for (bytes, key) in [(b"A", "a"), (b"B", "b")] {
        let mut engine = Engine::new();
        let empty = engine.empty().unwrap();
        let blob = engine.blob(bytes.to_vec()).unwrap();
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
        repo.commit_source(
            &engine,
            root,
            repo.status().unwrap().generation,
            &format!("source-{key}"),
        )
        .unwrap();
        revisions.push(
            repo.commit(
                "Author",
                key,
                repo.status().unwrap().generation,
                &format!("commit-{key}"),
            )
            .unwrap()
            .after
            .revision
            .unwrap(),
        );
    }
    (revisions[0], revisions[1])
}

#[test]
fn actual_cli_select_requires_experimental_gate_and_all_three_explicit_flags() {
    let project = tempfile::tempdir().unwrap();
    let p = project.path();
    let (a, _) = fixture(p);
    let id = a.to_string();
    let before = Repository::open(p).unwrap().status().unwrap();
    let cases = [
        vec![
            "history",
            "select",
            "--revision",
            &id,
            "--expected-generation",
            "4",
            "--request-key",
            "select",
        ],
        vec![
            "--experimental",
            "history",
            "select",
            "--expected-generation",
            "4",
            "--request-key",
            "select",
        ],
        vec![
            "--experimental",
            "history",
            "select",
            "--revision",
            &id,
            "--request-key",
            "select",
        ],
        vec![
            "--experimental",
            "history",
            "select",
            "--revision",
            &id,
            "--expected-generation",
            "4",
        ],
    ];
    for (args, required) in cases.iter().zip([
        "--experimental",
        "--revision",
        "--expected-generation",
        "--request-key",
    ]) {
        let output = command(p, args).output().unwrap();
        assert!(
            !output.status.success(),
            "accepted missing {required}: {output:?}"
        );
        assert!(
            text(&output).contains(required),
            "missing useful {required} diagnostic: {output:?}"
        );
        let repo = Repository::open(p).unwrap();
        assert_eq!(repo.status().unwrap(), before);
        assert!(repo.receipt("select").unwrap().is_none());
    }
    for args in [
        vec!["history", "--help"],
        vec!["history", "select", "--help"],
    ] {
        let output = success(p, &args);
        assert!(text(&output).contains("select"));
    }
    let help = text(&success(p, &["history", "select", "--help"]));
    for flag in ["--revision", "--expected-generation", "--request-key"] {
        assert!(help.contains(flag), "{help}");
    }
    assert!(!help.contains("--force"));
    assert!(!help.contains("--discard"));
    assert!(!help.contains("--stash"));
    let absent = p.join("must-not-be-initialized");
    let output = command(
        &absent,
        &[
            "--experimental",
            "history",
            "select",
            "--revision",
            &id,
            "--expected-generation",
            "0",
            "--request-key",
            "select",
        ],
    )
    .output()
    .unwrap();
    assert!(!output.status.success());
    assert!(
        !absent.exists(),
        "selection must not initialize a missing repository"
    );
}

#[test]
fn actual_cli_rejects_names_hostile_ids_and_dirty_selection_with_safe_diagnostics() {
    let project = tempfile::tempdir().unwrap();
    let p = project.path();
    let (a, _) = fixture(p);
    let mut repo = Repository::open(p).unwrap();
    repo.set_reference(&"saved".parse().unwrap(), a, 0, "save")
        .unwrap();
    drop(repo);
    for revision in ["saved", "HEAD", "main", "avc1-k05-NOT-CANONICAL"] {
        let output = command(
            p,
            &[
                "--experimental",
                "history",
                "select",
                "--revision",
                revision,
                "--expected-generation",
                "4",
                "--request-key",
                "bad-id",
            ],
        )
        .output()
        .unwrap();
        assert!(
            !output.status.success(),
            "accepted floating/noncanonical revision {revision}"
        );
    }
    // Reject hostile spelling at runtime without passing the rejected value
    // through usage-rs diagnostics (which may echo raw terminal controls).
    for revision in [
        "avc1-k05-\u{1b}[31mAVC_SELECTION_INJECTED",
        "avc1-k05-\nAVC_SELECTION_INJECTED",
    ] {
        let output = command(
            p,
            &[
                "--experimental",
                "history",
                "select",
                "--revision",
                revision,
                "--expected-generation",
                "4",
                "--request-key",
                "hostile-id",
            ],
        )
        .output()
        .unwrap();
        assert!(!output.status.success());
        let diagnostic = text(&output);
        assert!(
            diagnostic.contains("noncanonical ID spelling"),
            "{output:?}"
        );
        assert!(
            !diagnostic.contains('\u{1b}'),
            "raw terminal escape echoed: {output:?}"
        );
        assert!(
            !diagnostic.contains("AVC_SELECTION_INJECTED"),
            "rejected value echoed: {output:?}"
        );
        let repo = Repository::open(p).unwrap();
        assert_eq!(repo.status().unwrap().generation, 4);
        assert!(repo.receipt("hostile-id").unwrap().is_none());
    }
    let input = p.join("input");
    fs::write(&input, b"selected but uncommitted").unwrap();
    success(
        p,
        &[
            "--experimental",
            "source",
            "put",
            "file",
            "--file",
            input.to_str().unwrap(),
            "--expected-generation",
            "4",
            "--request-key",
            "dirty",
        ],
    );
    let before = Repository::open(p).unwrap().status().unwrap();
    let output = command(
        p,
        &[
            "--experimental",
            "history",
            "select",
            "--revision",
            &a.to_string(),
            "--expected-generation",
            "5",
            "--request-key",
            "select",
        ],
    )
    .output()
    .unwrap();
    assert!(!output.status.success());
    let diagnostic = text(&output);
    assert!(
        diagnostic.contains("selected-but-uncommitted source"),
        "{diagnostic}"
    );
    assert!(!diagnostic.contains("--force"));
    assert!(!diagnostic.contains("--discard"));
    assert!(!diagnostic.contains("--stash"));
    let repo = Repository::open(p).unwrap();
    assert_eq!(repo.status().unwrap(), before);
    assert!(repo.receipt("select").unwrap().is_none());
}

#[test]
fn actual_cli_selects_without_checkout_replays_receipt_and_executes_any_emitted_cancel_command() {
    let project = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let p = project.path();
    let (a, b) = fixture(p);
    fs::write(p.join("file"), b"live dirty and never captured").unwrap();
    fs::write(p.join(".git"), b"not a usable gitdir").unwrap();
    let before = Repository::open(p).unwrap().status().unwrap();
    let args = [
        "--experimental",
        "history",
        "select",
        "--revision",
        &a.to_string(),
        "--expected-generation",
        "4",
        "--request-key",
        "select-a",
    ];
    let output = command(p, &args)
        .current_dir(elsewhere.path())
        .env("PATH", elsewhere.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let diagnostic = text(&output);
    assert!(
        diagnostic.contains(&a.to_string()),
        "must identify the exact selected revision: {diagnostic}"
    );
    assert!(
        diagnostic.to_lowercase().contains("no checkout"),
        "{diagnostic}"
    );
    assert!(
        diagnostic.contains("live files") || diagnostic.contains("filesystem"),
        "{diagnostic}"
    );
    let repo = Repository::open(p).unwrap();
    let selected = repo.status().unwrap();
    assert_eq!(selected.revision, Some(a));
    assert_eq!(selected.root, repo.revision(a).unwrap().root);
    assert_eq!(selected.generation, before.generation + 1);
    let Some(Receipt::Selection(receipt)) = repo.receipt("select-a").unwrap() else {
        panic!("missing typed select receipt")
    };
    assert_eq!(receipt.operation, Operation::Select { revision: a });
    assert_eq!(receipt.before, before);
    assert_eq!(receipt.after, selected);
    drop(repo);
    // Move again, then retry the original request. Printed acknowledgement must
    // remain about its original before/after, not the current selection.
    success(
        p,
        &[
            "--experimental",
            "history",
            "select",
            "--revision",
            &b.to_string(),
            "--expected-generation",
            "5",
            "--request-key",
            "select-b",
        ],
    );
    let retry = success(p, &args);
    assert_eq!(retry.stdout, output.stdout);
    assert_eq!(retry.stderr, output.stderr);
    assert_eq!(
        Repository::open(p).unwrap().status().unwrap().revision,
        Some(b)
    );
    success(
        p,
        &[
            "--experimental",
            "history",
            "undo",
            "select-b",
            "--expected-generation",
            "6",
            "--request-key",
            "undo-b",
        ],
    );
    // Cancel hints are executable contracts, not unchecked prose. If a hint is
    // emitted, use it while the hinted selection still owns the generation.
    let cancel_output = success(
        p,
        &[
            "--experimental",
            "history",
            "select",
            "--revision",
            &a.to_string(),
            "--expected-generation",
            "7",
            "--request-key",
            "cancel-test",
        ],
    );
    let cancel_text = text(&cancel_output);
    if let Some(line) = cancel_text
        .lines()
        .find_map(|line| line.strip_prefix("Cancel: "))
    {
        let words = line.split_whitespace().collect::<Vec<_>>();
        assert_eq!(words.first(), Some(&"avc"));
        let args = words[1..]
            .iter()
            .map(|word| match *word {
                "SELECT_KEY" | "SELECTION_KEY" | "REQUEST_KEY" => "cancel-test",
                "NEW_KEY" => "cancel-undo",
                word => word,
            })
            .collect::<Vec<_>>();
        assert!(
            args.contains(&"--experimental"),
            "non-executable cancel hint: {line}"
        );
        assert!(
            args.contains(&"cancel-test"),
            "hint must name this operation: {line}"
        );
        success(p, &args);
        let repo = Repository::open(p).unwrap();
        assert_eq!(repo.status().unwrap().generation, 9);
        assert_eq!(repo.status().unwrap().revision, Some(a));
    }
    assert_eq!(
        fs::read(p.join("file")).unwrap(),
        b"live dirty and never captured"
    );
    assert_eq!(fs::read(p.join(".git")).unwrap(), b"not a usable gitdir");
    assert!(fs::read_dir(elsewhere.path()).unwrap().next().is_none());
}

#[test]
fn actual_cli_branch_export_follows_selected_ancestry_and_excludes_names_and_local_operations() {
    let project = tempfile::tempdir().unwrap();
    let p = project.path();
    let (a, b) = fixture(p);
    success(
        p,
        &[
            "--experimental",
            "ref",
            "set",
            "saved",
            "--revision",
            &b.to_string(),
            "--expected-generation",
            "0",
            "--request-key",
            "saved-ref",
        ],
    );
    success(
        p,
        &[
            "--experimental",
            "history",
            "select",
            "--revision",
            &a.to_string(),
            "--expected-generation",
            "4",
            "--request-key",
            "branch-from-a",
        ],
    );
    let input = p.join("input");
    fs::write(&input, b"C selected source").unwrap();
    success(
        p,
        &[
            "--experimental",
            "source",
            "put",
            "file",
            "--file",
            input.to_str().unwrap(),
            "--expected-generation",
            "5",
            "--request-key",
            "source-c",
        ],
    );
    fs::write(p.join("file"), b"live bytes are not committed").unwrap();
    success(
        p,
        &[
            "--experimental",
            "commit",
            "--author",
            "Branch author",
            "--message",
            "C",
            "--expected-generation",
            "6",
            "--request-key",
            "commit-c",
        ],
    );
    let repo = Repository::open(p).unwrap();
    let selected = repo.status().unwrap();
    let c = selected.revision.unwrap();
    assert_eq!(repo.revision(c).unwrap().parents, vec![a]);
    assert_eq!(
        repo.reference(&"saved".parse().unwrap())
            .unwrap()
            .unwrap()
            .revision,
        b
    );
    let history = p.join("branch.history");
    success(
        p,
        &["export", "--history", "--output", history.to_str().unwrap()],
    );
    let history_bytes = fs::read(&history).unwrap();
    assert_eq!(&history_bytes[..8], b"AVCHIS01");
    let source = p.join("branch.source");
    success(p, &["export", "--output", source.to_str().unwrap()]);
    assert_eq!(&fs::read(&source).unwrap()[..8], b"AVCEXP01");
    // Operations, generations and names must not affect canonical exports.
    success(
        p,
        &[
            "--experimental",
            "history",
            "select",
            "--revision",
            &c.to_string(),
            "--expected-generation",
            "7",
            "--request-key",
            "same-c",
        ],
    );
    success(
        p,
        &[
            "--experimental",
            "ref",
            "set",
            "saved",
            "--revision",
            &a.to_string(),
            "--expected-generation",
            "1",
            "--request-key",
            "move-saved",
        ],
    );
    let again = p.join("same.history");
    success(
        p,
        &["export", "--history", "--output", again.to_str().unwrap()],
    );
    assert_eq!(fs::read(again).unwrap(), history_bytes);
    let restored = tempfile::tempdir().unwrap();
    success(restored.path(), &["--experimental", "init"]);
    success(
        restored.path(),
        &[
            "--experimental",
            "import",
            "--history",
            "--input",
            history.to_str().unwrap(),
            "--request-key",
            "restore",
        ],
    );
    let imported = Repository::open(restored.path()).unwrap();
    assert_eq!(imported.status().unwrap().root, selected.root);
    assert_eq!(imported.status().unwrap().revision, Some(c));
    assert_eq!(
        imported
            .log(None, 20)
            .unwrap()
            .iter()
            .map(|(id, _)| *id)
            .collect::<Vec<_>>(),
        vec![c, a]
    );
    assert!(
        imported.revision(b).is_err(),
        "unselected B is not part of C's owning ancestry"
    );
    assert!(imported.references(None, 20).unwrap().is_empty());
    for key in [
        "branch-from-a",
        "saved-ref",
        "source-c",
        "commit-c",
        "same-c",
    ] {
        assert!(imported.receipt(key).unwrap().is_none());
    }
    assert_eq!(
        fs::read(p.join("file")).unwrap(),
        b"live bytes are not committed"
    );
}
