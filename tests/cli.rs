use avc::repository::Repository;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
fn run(project: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_avc"))
        .arg("--repository")
        .arg(project)
        .args(args)
        .output()
        .unwrap()
}
fn success(project: &Path, args: &[&str]) -> Output {
    let output = run(project, args);
    assert!(
        output.status.success(),
        "args={args:?}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
#[test]
fn cli_source_edit_reopen_export_and_independent_restore() {
    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join(".git")).unwrap();
    fs::write(project.path().join(".git/untouched"), b"foreign Git").unwrap();
    fs::write(project.path().join("user.txt"), b"live work").unwrap();
    assert!(!run(project.path(), &["init"]).status.success());
    assert!(!project.path().join(".avc").exists());
    success(project.path(), &["--experimental", "init"]);
    let initial = Repository::open(project.path()).unwrap().status().unwrap();
    assert_eq!(initial.generation, 0);
    let inputs = tempfile::tempdir().unwrap();
    let input = inputs.path().join("payload");
    let mut bytes = b"exact\0binary\r\n".to_vec();
    bytes.extend(vec![b'z'; 8192]);
    fs::write(&input, &bytes).unwrap();
    let input = input.to_str().unwrap();
    success(
        project.path(),
        &[
            "--experimental",
            "source",
            "put",
            "f",
            "--file",
            input,
            "--expected-generation",
            "0",
            "--request-key",
            "first",
        ],
    );
    let first = Repository::open(project.path()).unwrap().status().unwrap();
    assert_eq!(first.generation, 1);
    assert_eq!(success(project.path(), &["show", "f"]).stdout, bytes);
    assert!(
        !run(
            project.path(),
            &[
                "--experimental",
                "source",
                "remove",
                "f",
                "--expected-generation",
                "0",
                "--request-key",
                "stale"
            ]
        )
        .status
        .success()
    );
    success(
        project.path(),
        &[
            "--experimental",
            "source",
            "copy",
            "f",
            "g",
            "--expected-generation",
            "1",
            "--request-key",
            "second",
        ],
    );
    let second = Repository::open(project.path()).unwrap().status().unwrap();
    assert_eq!(second.generation, 2);
    // Retry after another selection advance must recover original outcome, not
    // reconstruct the write against today's selected source.
    success(
        project.path(),
        &[
            "--experimental",
            "source",
            "put",
            "f",
            "--file",
            input,
            "--expected-generation",
            "0",
            "--request-key",
            "first",
        ],
    );
    assert_eq!(
        Repository::open(project.path()).unwrap().status().unwrap(),
        second
    );
    fs::write(input, b"different retry").unwrap();
    assert!(
        !run(
            project.path(),
            &[
                "--experimental",
                "source",
                "put",
                "f",
                "--file",
                input,
                "--expected-generation",
                "0",
                "--request-key",
                "first"
            ]
        )
        .status
        .success()
    );
    let status = success(project.path(), &["status"]);
    assert!(String::from_utf8_lossy(&status.stdout).contains("not scanned"));
    success(project.path(), &["verify"]);
    success(project.path(), &["receipt", "first"]);
    let diff = success(
        project.path(),
        &["diff", &first.root.to_string(), &second.root.to_string()],
    );
    assert!(String::from_utf8_lossy(&diff.stdout).starts_with("g\t"));
    let export = inputs.path().join("source.avc");
    let export = export.to_str().unwrap();
    success(project.path(), &["export", "--output", export]);
    if std::env::var_os("AVC_REFERENCE_CHECK").is_some() {
        let output = Command::new("uv")
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .args(["run", "reference/bundle.py", export, "--json"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let independent: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            independent["roots"][0].as_str().unwrap(),
            second.root.to_string()
        );
    }
    assert!(
        !run(project.path(), &["export", "--output", export])
            .status
            .success()
    );
    let restored = tempfile::tempdir().unwrap();
    success(restored.path(), &["--experimental", "init"]);
    success(
        restored.path(),
        &[
            "--experimental",
            "import",
            "--input",
            export,
            "--request-key",
            "restore",
        ],
    );
    assert_eq!(success(restored.path(), &["show", "g"]).stdout, bytes);
    assert_eq!(
        Repository::open(restored.path())
            .unwrap()
            .status()
            .unwrap()
            .root,
        second.root
    );
    let output = inputs.path().canonicalize().unwrap().join("fresh-source");
    success(
        restored.path(),
        &[
            "--experimental",
            "restore",
            "--to",
            output.to_str().unwrap(),
        ],
    );
    assert_eq!(fs::read(output.join("f")).unwrap(), bytes);
    assert_eq!(fs::read(output.join("g")).unwrap(), bytes);
    assert!(
        !run(
            restored.path(),
            &[
                "--experimental",
                "restore",
                "--to",
                output.to_str().unwrap()
            ]
        )
        .status
        .success()
    );
    assert_eq!(fs::read(output.join("f")).unwrap(), bytes);
    assert_eq!(
        fs::read(project.path().join("user.txt")).unwrap(),
        b"live work"
    );
    assert_eq!(
        fs::read(project.path().join(".git/untouched")).unwrap(),
        b"foreign Git"
    );
}
#[test]
fn cli_legacy_and_corrupt_export_fail_without_selection_changes() {
    let legacy = tempfile::tempdir().unwrap();
    fs::create_dir(legacy.path().join(".avc")).unwrap();
    fs::write(
        legacy.path().join(".avc/recovery.json"),
        b"old protected work",
    )
    .unwrap();
    assert!(
        !run(legacy.path(), &["--experimental", "init"])
            .status
            .success()
    );
    assert!(!run(legacy.path(), &["status"]).status.success());
    assert_eq!(
        fs::read(legacy.path().join(".avc/recovery.json")).unwrap(),
        b"old protected work"
    );
    let project = tempfile::tempdir().unwrap();
    success(project.path(), &["--experimental", "init"]);
    let before = Repository::open(project.path()).unwrap().status().unwrap();
    let file = project.path().join("bad.avc");
    fs::write(&file, b"AVCEXP01truncated").unwrap();
    assert!(
        !run(
            project.path(),
            &[
                "--experimental",
                "import",
                "--input",
                file.to_str().unwrap(),
                "--request-key",
                "bad"
            ]
        )
        .status
        .success()
    );
    assert_eq!(
        Repository::open(project.path()).unwrap().status().unwrap(),
        before
    );
}
#[test]
fn cli_rejects_source_path_escape_and_preserves_live_files() {
    let project = tempfile::tempdir().unwrap();
    success(project.path(), &["--experimental", "init"]);
    let input = project.path().join("input");
    fs::write(&input, b"data").unwrap();
    for name in ["../outside", "/absolute", "a//b", "."] {
        assert!(
            !run(
                project.path(),
                &[
                    "--experimental",
                    "source",
                    "put",
                    name,
                    "--file",
                    input.to_str().unwrap(),
                    "--expected-generation",
                    "0",
                    "--request-key",
                    "invalid"
                ]
            )
            .status
            .success()
        );
    }
    assert_eq!(
        Repository::open(project.path())
            .unwrap()
            .status()
            .unwrap()
            .generation,
        0
    );
    success(
        project.path(),
        &[
            "--experimental",
            "source",
            "mkdir",
            "dir",
            "--expected-generation",
            "0",
            "--request-key",
            "directory",
        ],
    );
    success(
        project.path(),
        &[
            "--experimental",
            "source",
            "put",
            "dir/f",
            "--file",
            input.to_str().unwrap(),
            "--expected-generation",
            "1",
            "--request-key",
            "file",
        ],
    );
    assert_eq!(success(project.path(), &["show", "dir/f"]).stdout, b"data");
}
