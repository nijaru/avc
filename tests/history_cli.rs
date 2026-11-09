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
