#![cfg(unix)]
use avc::repository::Repository;
use std::{fs, path::Path, process::Command};
fn run(project: &Path, input: &Path, import: bool) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_avc"));
    command
        .arg("--repository")
        .arg(project)
        .arg("--experimental");
    if import {
        command
            .args(["import", "--input"])
            .arg(input)
            .args(["--request-key", "input"]);
    } else {
        command
            .args(["source", "put", "data", "--file"])
            .arg(input)
            .args(["--expected-generation", "0", "--request-key", "input"]);
    }
    command.output().unwrap()
}
#[test]
fn cli_byte_suppliers_refuse_native_administration_before_opening_database_inputs() {
    for import in [false, true] {
        let project = tempfile::tempdir().unwrap();
        let repo = Repository::init(project.path()).unwrap();
        let before = repo.status().unwrap();
        let input = project.path().join(".avc/store.sqlite");
        let output = run(project.path(), &input, import);
        assert!(
            !output.status.success(),
            "must refuse an administration byte supplier"
        );
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains("administrat"), "{error}");
        assert_eq!(repo.status().unwrap(), before);
        assert!(repo.receipt("input").unwrap().is_none());
    }
}
#[test]
fn cli_byte_suppliers_refuse_symlink_and_hardlink_aliases_of_native_database() {
    for import in [false, true] {
        for hardlink in [false, true] {
            let project = tempfile::tempdir().unwrap();
            let repo = Repository::init(project.path()).unwrap();
            let before = repo.status().unwrap();
            let input = project.path().join("alias");
            let database = project.path().join(".avc/store.sqlite");
            if hardlink {
                fs::hard_link(&database, &input).unwrap();
            } else {
                std::os::unix::fs::symlink(&database, &input).unwrap();
            }
            let output = run(project.path(), &input, import);
            assert!(!output.status.success(), "must refuse a database alias");
            let error = String::from_utf8(output.stderr).unwrap();
            assert!(
                error.contains("administrat") || error.contains("hardlink"),
                "{error}"
            );
            assert_eq!(repo.status().unwrap(), before);
            assert!(repo.receipt("input").unwrap().is_none());
        }
    }
}
#[test]
fn ordinary_symlink_byte_suppliers_preserve_source_and_bundle_input_behavior() {
    for import in [false, true] {
        let project = tempfile::tempdir().unwrap();
        let input = tempfile::tempdir().unwrap();
        let mut repo = Repository::init(project.path()).unwrap();
        fs::create_dir_all(project.path().join(".avc/exports")).unwrap();
        let source = project.path().join(".avc/exports/ordinary");
        let alias = input.path().join("alias");
        if import {
            repo.export_file(repo.status().unwrap().root, &source)
                .unwrap();
        } else {
            fs::write(&source, b"binary\0\xff\r\n").unwrap();
        }
        std::os::unix::fs::symlink(&source, &alias).unwrap();
        let output = run(project.path(), &alias, import);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(repo.status().unwrap().generation, 1);
        assert!(repo.receipt("input").unwrap().is_some());
        if !import {
            let root = repo.status().unwrap().root;
            let engine = repo.load_root(root).unwrap();
            let avc::Entry::File { blob, .. } = engine.read(root, &[b"data".to_vec()]).unwrap()
            else {
                panic!("not file");
            };
            let avc::Object::Blob(bytes) = engine.object(blob).unwrap() else {
                panic!("not blob");
            };
            assert_eq!(bytes, b"binary\0\xff\r\n");
        }
        // A later ordinary capture still operates normally after input reading.
        fs::write(project.path().join("later"), b"later").unwrap();
        repo.capture(
            &avc::repository::CaptureScope::new(project.path(), vec![vec![b"later".to_vec()]])
                .unwrap(),
            1,
            "later",
        )
        .unwrap();
    }
}
#[test]
fn fifo_bundle_input_refuses_without_waiting_for_a_writer() {
    let project = tempfile::tempdir().unwrap();
    let repo = Repository::init(project.path()).unwrap();
    let input = project.path().join("fifo");
    assert!(
        Command::new("mkfifo")
            .arg(&input)
            .status()
            .unwrap()
            .success()
    );
    let output = run(project.path(), &input, true);
    assert!(!output.status.success());
    assert_eq!(repo.status().unwrap().generation, 0);
}
