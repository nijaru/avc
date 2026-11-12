use avc::{Object, repository::Repository};
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
    let output = cli(project, args).output().unwrap();
    assert!(output.status.success(), "{args:?}: {output:?}");
    output
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn actual_cli_authors_selected_source_and_reads_persisted_history() {
    let project = tempfile::tempdir().unwrap();
    let input = project.path().join("input");
    fs::write(&input, b"first selected bytes").unwrap();
    let live = project.path().join("live.txt");
    fs::write(&live, b"not selected and never captured").unwrap();
    success(project.path(), &["--experimental", "init"]);
    assert!(
        String::from_utf8(success(project.path(), &["status"]).stdout)
            .unwrap()
            .contains("Revision: unborn")
    );
    success(
        project.path(),
        &[
            "--experimental",
            "source",
            "put",
            "file",
            "--file",
            input.to_str().unwrap(),
            "--expected-generation",
            "0",
            "--request-key",
            "select-one",
        ],
    );
    let rejected = cli(
        project.path(),
        &[
            "commit",
            "--author",
            "Ada",
            "-m",
            "one",
            "--expected-generation",
            "1",
            "--request-key",
            "author-one",
        ],
    )
    .output()
    .unwrap();
    assert!(!rejected.status.success());
    assert_eq!(
        Repository::open(project.path())
            .unwrap()
            .status()
            .unwrap()
            .revision,
        None
    );
    let first_output = success(
        project.path(),
        &[
            "--experimental",
            "commit",
            "--author",
            "Ada",
            "-m",
            "one",
            "--expected-generation",
            "1",
            "--request-key",
            "author-one",
        ],
    );
    let first = Repository::open(project.path())
        .unwrap()
        .status()
        .unwrap()
        .revision
        .unwrap();
    assert!(
        String::from_utf8(first_output.stdout.clone())
            .unwrap()
            .contains(&format!("revision={first}"))
    );
    fs::write(&input, b"second selected bytes").unwrap();
    success(
        project.path(),
        &[
            "--experimental",
            "source",
            "put",
            "file",
            "--file",
            input.to_str().unwrap(),
            "--expected-generation",
            "2",
            "--request-key",
            "select-two",
        ],
    );
    let message = "second\nline\u{1b}[31m";
    let second_output = cli(
        project.path(),
        &[
            "--experimental",
            "commit",
            "-m",
            message,
            "--expected-generation",
            "3",
            "--request-key",
            "author-two",
        ],
    )
    .env("AVC_AUTHOR", "Env Author")
    .output()
    .unwrap();
    assert!(second_output.status.success(), "{second_output:?}");
    let repo = Repository::open(project.path()).unwrap();
    let selected = repo.status().unwrap();
    let second = selected.revision.unwrap();
    assert_eq!(selected.generation, 4);
    assert_eq!(repo.revision(second).unwrap().parents, vec![first]);
    assert_eq!(repo.revision(second).unwrap().author, "Env Author");
    let log = String::from_utf8(success(project.path(), &["log"]).stdout).unwrap();
    assert!(
        log.find(&format!("Revision: {second}")).unwrap()
            < log.find(&format!("Revision: {first}")).unwrap()
    );
    assert!(log.contains("Author: Env Author"));
    assert!(log.contains("Message: second\\nline\\u{1b}[31m"));
    assert!(!log.contains('\u{1b}'));
    let limited =
        String::from_utf8(success(project.path(), &["log", "--limit", "1"]).stdout).unwrap();
    assert!(limited.contains(&format!("Revision: {second}")));
    assert!(!limited.contains(&format!("Revision: {first}")));
    let retry = success(
        project.path(),
        &[
            "--experimental",
            "commit",
            "--author",
            "Ada",
            "-m",
            "one",
            "--expected-generation",
            "1",
            "--request-key",
            "author-one",
        ],
    );
    assert_eq!(retry.stdout, first_output.stdout);
    assert_eq!(repo.status().unwrap(), selected);
    success(project.path(), &["verify", "--root", &second.to_string()]);
    assert_eq!(
        success(project.path(), &["show", "file"]).stdout,
        b"second selected bytes"
    );
    assert!(
        !cli(project.path(), &["show", "live.txt"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(fs::read(live).unwrap(), b"not selected and never captured");

    if std::env::var_os("AVC_REFERENCE_CHECK").is_some() {
        // Validate actual CLI-authored revision bytes/parents with the independent
        // codec, not just a hand-built Revision fixture or the Rust decoder.
        let engine = repo.load_root(second).unwrap();
        let objects = engine.objects().map(|(id, object)| {
            serde_json::json!({"id": id.to_string(), "payload": hex(&object.encode().unwrap())})
        }).collect::<Vec<_>>();
        let corpus = project.path().join("history.json");
        fs::write(
            &corpus,
            serde_json::to_vec(
                &serde_json::json!({"objects": objects, "roots": [second.to_string()]}),
            )
            .unwrap(),
        )
        .unwrap();
        let output = Command::new("uv")
            .args(["run", "reference/check.py", "--verify"])
            .arg(corpus)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "independent revision verification: {output:?}"
        );
        assert!(
            engine
                .objects()
                .any(|(_, object)| matches!(object, Object::Revision(_)))
        );
    }
}

#[test]
fn actual_cli_amends_pinned_source_and_undoes_only_local_selection() {
    let project = tempfile::tempdir().unwrap();
    let p = project.path();
    let input = p.join("input");
    fs::write(&input, b"selected bytes").unwrap();
    success(p, &["--experimental", "init"]);
    success(
        p,
        &[
            "--experimental",
            "commit",
            "--author",
            "Author",
            "-m",
            "empty",
            "--expected-generation",
            "0",
            "--request-key",
            "first",
        ],
    );
    let original = Repository::open(p).unwrap().status().unwrap();
    let empty = success(
        p,
        &[
            "--experimental",
            "amend",
            "--author",
            "Author",
            "-m",
            "empty",
            "--expected-generation",
            "1",
            "--request-key",
            "empty-amend",
        ],
    );
    let diagnostic = String::from_utf8(empty.stderr).unwrap();
    assert!(diagnostic.contains("Source unchanged"));
    assert!(diagnostic.contains("Empty change"));
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
            "2",
            "--request-key",
            "working",
        ],
    );
    fs::write(&input, b"later live bytes, never selected").unwrap();
    assert!(
        !cli(
            p,
            &[
                "amend",
                "--author",
                "Author",
                "-m",
                "new",
                "--expected-generation",
                "3",
                "--request-key",
                "amend"
            ]
        )
        .output()
        .unwrap()
        .status
        .success()
    );
    let output = success(
        p,
        &[
            "--experimental",
            "amend",
            "--author",
            "New Author",
            "-m",
            "new",
            "--expected-generation",
            "3",
            "--request-key",
            "amend",
        ],
    );
    let repo = Repository::open(p).unwrap();
    let amended = repo.status().unwrap();
    let revision = repo.revision(amended.revision.unwrap()).unwrap();
    assert_eq!(
        revision.change,
        repo.revision(original.revision.unwrap()).unwrap().change
    );
    assert!(revision.parents.is_empty());
    assert_eq!(revision.author, "New Author");
    assert_eq!(success(p, &["show", "file"]).stdout, b"selected bytes");
    let unchanged = success(
        p,
        &[
            "--experimental",
            "amend",
            "--author",
            "New Author",
            "-m",
            "new",
            "--expected-generation",
            "4",
            "--request-key",
            "unchanged",
        ],
    );
    assert!(
        String::from_utf8(unchanged.stderr.clone())
            .unwrap()
            .contains("Source unchanged")
    );
    let before_undo = repo.status().unwrap();
    assert_ne!(before_undo.revision, amended.revision);
    assert!(
        !cli(
            p,
            &[
                "history",
                "undo",
                "unchanged",
                "--expected-generation",
                "5",
                "--request-key",
                "undo"
            ]
        )
        .output()
        .unwrap()
        .status
        .success()
    );
    let undo = success(
        p,
        &[
            "--experimental",
            "history",
            "undo",
            "unchanged",
            "--expected-generation",
            "5",
            "--request-key",
            "undo",
        ],
    );
    assert!(
        String::from_utf8(undo.stderr)
            .unwrap()
            .contains("filesystem and remotes unchanged")
    );
    let after_undo = repo.status().unwrap();
    assert_eq!(after_undo.generation, 6);
    assert_eq!(
        (after_undo.root, after_undo.revision),
        (amended.root, amended.revision)
    );
    assert_eq!(
        fs::read(&input).unwrap(),
        b"later live bytes, never selected"
    );
    assert_eq!(
        success(
            p,
            &[
                "--experimental",
                "amend",
                "--author",
                "New Author",
                "-m",
                "new",
                "--expected-generation",
                "3",
                "--request-key",
                "amend"
            ]
        )
        .stdout,
        output.stdout
    );
    assert_eq!(
        success(
            p,
            &[
                "--experimental",
                "amend",
                "--author",
                "New Author",
                "-m",
                "new",
                "--expected-generation",
                "4",
                "--request-key",
                "unchanged"
            ]
        )
        .stdout,
        unchanged.stdout
    );
    assert_eq!(repo.status().unwrap(), after_undo);
    assert!(
        !cli(
            p,
            &[
                "--experimental",
                "history",
                "undo",
                "amend",
                "--expected-generation",
                "6",
                "--request-key",
                "old-operation"
            ]
        )
        .output()
        .unwrap()
        .status
        .success()
    );
    success(
        p,
        &["verify", "--root", &amended.revision.unwrap().to_string()],
    );
    let bundle = p.join("history.avc");
    success(
        p,
        &["export", "--history", "--output", bundle.to_str().unwrap()],
    );
    if std::env::var_os("AVC_REFERENCE_CHECK").is_some() {
        let check = Command::new("uv")
            .args(["run", "reference/bundle.py", "--history"])
            .arg(bundle)
            .output()
            .unwrap();
        assert!(
            check.status.success(),
            "independent amended-history verification: {check:?}"
        );
    }
}

#[test]
fn committed_amend_retry_is_not_failed_by_unavailable_comparison_diagnostics() {
    let project = tempfile::tempdir().unwrap();
    let p = project.path();
    let mut repo = Repository::init(p).unwrap();
    let parent = repo
        .commit("Author", "parent", 0, "parent")
        .unwrap()
        .after
        .revision
        .unwrap();
    repo.commit("Author", "tip", 1, "tip").unwrap();
    let args = [
        "--experimental",
        "amend",
        "--author",
        "Author",
        "-m",
        "amend",
        "--expected-generation",
        "2",
        "--request-key",
        "amend",
    ];
    let acknowledged = success(p, &args);
    let current = repo.status().unwrap();
    let mut damaged = Object::Revision(repo.revision(parent).unwrap())
        .encode()
        .unwrap();
    *damaged.last_mut().unwrap() ^= 1;
    let db = rusqlite::Connection::open(p.join(".avc/store.sqlite")).unwrap();
    db.execute_batch("DROP TRIGGER objects_no_update").unwrap();
    db.execute(
        "UPDATE objects SET payload=?1 WHERE id=?2",
        rusqlite::params![damaged, parent.to_bytes().unwrap().as_slice()],
    )
    .unwrap();
    let retry = success(p, &args);
    assert_eq!(retry.stdout, acknowledged.stdout);
    assert!(
        String::from_utf8(retry.stderr)
            .unwrap()
            .contains("comparison diagnostics unavailable")
    );
    assert_eq!(repo.status().unwrap(), current);
}
