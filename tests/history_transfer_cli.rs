use avc::repository::Repository;
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
fn run(project: &Path, args: &[&str]) -> Output {
    let output = command(project, args).output().unwrap();
    assert!(output.status.success(), "{args:?}: {output:?}");
    output
}

#[test]
fn cli_history_is_self_contained_and_explicitly_separate_from_source_transfer() {
    let scratch = tempfile::tempdir().unwrap();
    let original = scratch.path().join("original");
    let fresh = scratch.path().join("fresh");
    fs::create_dir(&original).unwrap();
    fs::create_dir(&fresh).unwrap();
    let input = scratch.path().join("input");
    fs::write(&input, b"authored bytes\0\r\n").unwrap();
    run(&original, &["--experimental", "init"]);
    run(
        &original,
        &[
            "--experimental",
            "source",
            "put",
            "file",
            "--file",
            input.to_str().unwrap(),
            "--executable",
            "--expected-generation",
            "0",
            "--request-key",
            "source",
        ],
    );
    run(
        &original,
        &[
            "--experimental",
            "commit",
            "--author",
            "Author",
            "-m",
            "exact authored metadata",
            "--expected-generation",
            "1",
            "--request-key",
            "author",
        ],
    );
    let log = run(&original, &["log"]).stdout;
    fs::write(&input, b"selected but not authored").unwrap();
    run(
        &original,
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
    let selected = Repository::open(&original).unwrap().status().unwrap();
    let history = scratch.path().join("history.avc");
    let output = run(
        &original,
        &["export", "--history", "--output", history.to_str().unwrap()],
    );
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("derived Git maps excluded")
    );
    let source = scratch.path().join("source.avc");
    run(&original, &["export", "--output", source.to_str().unwrap()]);
    assert_eq!(&fs::read(&history).unwrap()[..8], b"AVCHIS01");
    assert_eq!(&fs::read(&source).unwrap()[..8], b"AVCEXP01");
    let invalid = scratch.path().join("must-not-create.avc");
    assert!(
        !command(
            &original,
            &[
                "export",
                "--history",
                "--root",
                &selected.root.to_string(),
                "--output",
                invalid.to_str().unwrap()
            ]
        )
        .output()
        .unwrap()
        .status
        .success()
    );
    assert!(!invalid.exists());
    assert!(
        !command(
            &original,
            &["export", "--history", "--output", history.to_str().unwrap()]
        )
        .output()
        .unwrap()
        .status
        .success()
    );
    // The restored result cannot depend on opening the original store.
    fs::remove_dir_all(&original).unwrap();
    run(&fresh, &["--experimental", "init"]);
    assert!(
        !command(
            &fresh,
            &[
                "import",
                "--history",
                "--input",
                history.to_str().unwrap(),
                "--request-key",
                "restore"
            ]
        )
        .output()
        .unwrap()
        .status
        .success()
    );
    assert!(
        !command(
            &fresh,
            &[
                "--experimental",
                "import",
                "--input",
                history.to_str().unwrap(),
                "--request-key",
                "restore"
            ]
        )
        .output()
        .unwrap()
        .status
        .success()
    );
    assert!(
        !command(
            &fresh,
            &[
                "--experimental",
                "import",
                "--history",
                "--input",
                source.to_str().unwrap(),
                "--request-key",
                "restore"
            ]
        )
        .output()
        .unwrap()
        .status
        .success()
    );
    assert_eq!(
        Repository::open(&fresh)
            .unwrap()
            .status()
            .unwrap()
            .generation,
        0
    );
    let receipt = run(
        &fresh,
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
    let restored = Repository::open(&fresh).unwrap().status().unwrap();
    assert_eq!(
        (restored.root, restored.revision, restored.generation),
        (selected.root, selected.revision, 1)
    );
    assert_eq!(run(&fresh, &["log"]).stdout, log);
    assert_eq!(
        run(&fresh, &["show", "file"]).stdout,
        b"selected but not authored"
    );
    run(
        &fresh,
        &["verify", "--root", &restored.revision.unwrap().to_string()],
    );
    // macOS /var is a symlink; restore intentionally refuses symlink parents.
    let materialized = fs::canonicalize(scratch.path())
        .unwrap()
        .join("materialized");
    run(
        &fresh,
        &[
            "--experimental",
            "restore",
            "--to",
            materialized.to_str().unwrap(),
        ],
    );
    assert_eq!(
        fs::read(materialized.join("file")).unwrap(),
        b"selected but not authored"
    );
    let retry = run(
        &fresh,
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
    assert_eq!(retry.stdout, receipt.stdout);
    let reexport = scratch.path().join("reexport.avc");
    run(
        &fresh,
        &[
            "export",
            "--history",
            "--output",
            reexport.to_str().unwrap(),
        ],
    );
    assert_eq!(fs::read(reexport).unwrap(), fs::read(&history).unwrap());

    if std::env::var_os("AVC_REFERENCE_CHECK").is_some() {
        let output = Command::new("uv")
            .args(["run", "reference/bundle.py", "--history", "--json"])
            .arg(&history)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "independent actual-history stream verification: {output:?}"
        );
        let corpus: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            corpus["roots"],
            serde_json::json!([
                selected.root.to_string(),
                selected.revision.unwrap().to_string()
            ])
        );
        let source_output = Command::new("uv")
            .args(["run", "reference/bundle.py"])
            .arg(source)
            .output()
            .unwrap();
        assert!(
            source_output.status.success(),
            "source-only framing changed: {source_output:?}"
        );
    }
}
