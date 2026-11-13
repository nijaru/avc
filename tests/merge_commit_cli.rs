//! Actual argv parsing and runtime merge-authoring contract, not parser mocks.
use avc::{Id, repository::Repository};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn cli(project: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_avc"));
    command
        .env_remove("AVC_AUTHOR")
        .arg("--repository")
        .arg(project)
        .args(args);
    command
}
fn success(project: &Path, args: &[&str]) -> Output {
    let out = cli(project, args).output().unwrap();
    assert!(out.status.success(), "{args:?}: {out:?}");
    out
}
fn fixture() -> (tempfile::TempDir, Id, Id) {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(dir.path()).unwrap();
    let parent = repo
        .commit("Initial Author", "parent", 0, "parent")
        .unwrap()
        .after
        .revision
        .unwrap();
    let head = repo
        .commit("Initial Author", "head", 1, "head")
        .unwrap()
        .after
        .revision
        .unwrap();
    (dir, head, parent)
}
fn request<'a>(parent: &'a str, key: &'a str) -> Vec<&'a str> {
    vec![
        "--experimental",
        "commit",
        "--merge-parent",
        parent,
        "--author",
        "CLI Author",
        "--message",
        "merge message",
        "--expected-generation",
        "2",
        "--request-key",
        key,
    ]
}

#[test]
fn commit_help_exposes_optional_merge_parent_but_amend_neither_advertises_nor_accepts_it() {
    let (dir, _, parent) = fixture();
    let commit = success(dir.path(), &["commit", "--help"]);
    assert!(
        String::from_utf8(commit.stdout)
            .unwrap()
            .contains("--merge-parent")
    );
    let amend = success(dir.path(), &["amend", "--help"]);
    assert!(
        !String::from_utf8(amend.stdout)
            .unwrap()
            .contains("--merge-parent")
    );
    let parent = parent.to_string();
    let mut args = request(&parent, "amend-reject");
    args[1] = "amend";
    let before = Repository::open(dir.path()).unwrap().status().unwrap();
    let out = cli(dir.path(), &args).output().unwrap();
    assert!(!out.status.success(), "{out:?}");
    let repo = Repository::open(dir.path()).unwrap();
    assert_eq!(repo.status().unwrap(), before);
    assert!(repo.receipt("amend-reject").unwrap().is_none());
}

#[test]
fn merge_commit_uses_author_env_required_fields_and_exact_selected_source_and_prints_parent() {
    let (dir, head, parent) = fixture();
    fs::write(dir.path().join("live"), b"never capture or overwrite").unwrap();
    let parent_text = parent.to_string();
    let message = "merge\nmessage\u{1b}[31m";
    let args = [
        "--experimental",
        "commit",
        "--merge-parent",
        &parent_text,
        "--message",
        message,
        "--expected-generation",
        "2",
        "--request-key",
        "merge-env",
    ];
    let before = Repository::open(dir.path()).unwrap().status().unwrap();
    let out = cli(dir.path(), &args)
        .env("AVC_AUTHOR", "Environment Author")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8(out.stdout.clone()).unwrap();
    assert!(
        stdout.contains(&format!("merge-parent={parent}")),
        "{stdout}"
    );
    assert!(!stdout.contains('\u{1b}'));
    let repo = Repository::open(dir.path()).unwrap();
    let after = repo.status().unwrap();
    assert_eq!(after.root, before.root);
    assert_eq!(after.generation, before.generation + 1);
    let revision = repo.revision(after.revision.unwrap()).unwrap();
    assert_eq!(revision.parents, vec![head, parent]);
    assert_eq!(revision.author, "Environment Author");
    assert_eq!(revision.message, message);
    assert_eq!(
        fs::read(dir.path().join("live")).unwrap(),
        b"never capture or overwrite"
    );
    let replay = cli(dir.path(), &args)
        .env("AVC_AUTHOR", "Environment Author")
        .output()
        .unwrap();
    assert!(replay.status.success(), "{replay:?}");
    assert_eq!(replay.stdout, out.stdout);
    assert_eq!(repo.status().unwrap(), after);
    let log = success(dir.path(), &["log", "--limit", "1"]);
    let log = String::from_utf8(log.stdout).unwrap();
    assert!(log.contains(&head.to_string()));
    assert!(log.contains(&parent.to_string()));
    assert!(!log.contains('\u{1b}'));
}

#[test]
fn ordinary_commit_remains_available_without_optional_parent_and_author_flag_overrides_env() {
    let (dir, head, _) = fixture();
    let out = cli(
        dir.path(),
        &[
            "--experimental",
            "commit",
            "--author",
            "Explicit Author",
            "--message",
            "ordinary",
            "--expected-generation",
            "2",
            "--request-key",
            "ordinary",
        ],
    )
    .env("AVC_AUTHOR", "Environment Author")
    .output()
    .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(
        !String::from_utf8(out.stdout)
            .unwrap()
            .contains("merge-parent=")
    );
    let repo = Repository::open(dir.path()).unwrap();
    let revision = repo
        .revision(repo.status().unwrap().revision.unwrap())
        .unwrap();
    assert_eq!(revision.parents, vec![head]);
    assert_eq!(revision.author, "Explicit Author");
}

#[test]
fn experimental_gate_and_each_required_old_metadata_field_still_apply_to_merge_commit() {
    let (dir, _, parent) = fixture();
    let parent = parent.to_string();
    let before = Repository::open(dir.path()).unwrap().status().unwrap();
    for missing in [
        "--experimental",
        "--author",
        "--message",
        "--expected-generation",
        "--request-key",
    ] {
        let mut args = request(&parent, "missing-field");
        let index = args.iter().position(|arg| *arg == missing).unwrap();
        args.remove(index);
        if missing != "--experimental" {
            args.remove(index);
        }
        let out = cli(dir.path(), &args).output().unwrap();
        assert!(!out.status.success(), "accepted missing {missing}: {out:?}");
        let repo = Repository::open(dir.path()).unwrap();
        assert_eq!(repo.status().unwrap(), before);
        assert!(repo.receipt("missing-field").unwrap().is_none());
    }
}

#[test]
fn invalid_runtime_parent_id_reports_terminal_safe_error_and_never_publishes() {
    let (dir, _, parent) = fixture();
    let before = Repository::open(dir.path()).unwrap().status().unwrap();
    for value in [
        "not-an-id".to_owned(),
        format!("{parent}\nINJECTED\u{1b}[31m"),
    ] {
        // Supply every old required field: this must exercise the new runtime
        // String-to-Id path, not an unrelated missing-metadata parser error.
        let out = cli(dir.path(), &request(&value, "invalid-parent"))
            .output()
            .unwrap();
        assert!(!out.status.success(), "{out:?}");
        let stderr = String::from_utf8(out.stderr).unwrap();
        assert!(
            !stderr.contains('\u{1b}'),
            "raw terminal escape: {stderr:?}"
        );
        assert!(
            !stderr.contains("\nINJECTED"),
            "raw injected newline: {stderr:?}"
        );
        assert!(
            !stderr.contains("unexpected argument '--merge-parent'"),
            "flag was not recognized: {stderr}"
        );
        let repo = Repository::open(dir.path()).unwrap();
        assert_eq!(repo.status().unwrap(), before);
        assert!(repo.receipt("invalid-parent").unwrap().is_none());
    }
}
