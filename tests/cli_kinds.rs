#![cfg(unix)]
use std::{fs, os::unix::fs::PermissionsExt, process::Command};
#[test]
fn cli_materializes_empty_directory_executable_and_literal_symlink() {
    let project = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    let input = parent.path().join("payload");
    fs::write(&input, b"#!/bin/sh\nexit 0\n").unwrap();
    let avc = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_avc"))
            .arg("--repository")
            .arg(project.path())
            .arg("--experimental")
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    };
    avc(&["init"]);
    avc(&[
        "source",
        "put",
        "run",
        "--file",
        input.to_str().unwrap(),
        "--executable",
        "--expected-generation",
        "0",
        "--request-key",
        "run",
    ]);
    fs::write(&input, b"run").unwrap();
    avc(&[
        "source",
        "put",
        "link",
        "--file",
        input.to_str().unwrap(),
        "--symlink",
        "--expected-generation",
        "1",
        "--request-key",
        "link",
    ]);
    avc(&[
        "source",
        "mkdir",
        "empty",
        "--expected-generation",
        "2",
        "--request-key",
        "empty",
    ]);
    let output = parent.path().canonicalize().unwrap().join("source");
    avc(&["restore", "--to", output.to_str().unwrap()]);
    assert_eq!(
        fs::read(output.join("run")).unwrap(),
        b"#!/bin/sh\nexit 0\n"
    );
    assert_eq!(
        fs::metadata(output.join("run"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    assert_eq!(
        fs::read_link(output.join("link")).unwrap(),
        std::path::Path::new("run")
    );
    assert!(output.join("empty").is_dir());
    assert!(!output.join(".avc").exists());
}
