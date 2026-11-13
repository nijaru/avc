use avc::repository::Repository;
use std::{
    path::Path,
    process::{Command, Output},
};

fn cli(project: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_avc"))
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

fn text(output: Output) -> String {
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn actual_cli_sets_a_named_revision_without_changing_selected_source() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let selected = repo.commit("Author", "first", 0, "commit").unwrap().after;
    let revision = selected.revision.unwrap().to_string();
    // Ref generation is zero even though selected-source generation is one.
    let args = [
        "--experimental",
        "ref",
        "set",
        "release/v1",
        "--revision",
        &revision,
        "--expected-generation",
        "0",
        "--request-key",
        "publish-name",
    ];
    let accepted = success(project.path(), &args);
    assert_eq!(repo.status().unwrap(), selected);
    let receipt = String::from_utf8(accepted.stdout.clone()).unwrap();
    for field in ["release/v1", &revision, "generation=1", "publish-name"] {
        assert!(receipt.contains(field), "missing {field:?}: {receipt}");
    }
    assert!(
        !receipt.to_ascii_lowercase().contains("root"),
        "ref receipt must not invent a source root: {receipt}"
    );
    let shown = text(success(project.path(), &["ref", "show", "release/v1"]));
    assert!(shown.contains(&revision));
    assert!(shown.contains("release/v1"));
    assert_eq!(success(project.path(), &args).stdout, accepted.stdout);
    assert_eq!(
        success(project.path(), &["receipt", "publish-name"]).stdout,
        accepted.stdout
    );
    let listed = text(success(project.path(), &["ref", "list"]));
    assert!(listed.contains("release/v1"));
    assert!(listed.contains(&revision));
    assert_eq!(repo.status().unwrap(), selected);
}

#[test]
fn invalid_ref_name_diagnostics_never_echo_input_controls() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let selected = repo.commit("Author", "first", 0, "commit").unwrap().after;
    let revision = selected.revision.unwrap().to_string();
    for invalid in [
        "bad\u{1b}[2J\nFORGED",
        "bad\nFORGED",
        "bad\u{7f}label",
        "bad\u{202e} label",
    ] {
        for args in [
            vec!["ref", "show", invalid],
            vec!["ref", "list", "--after", invalid],
            vec![
                "--experimental",
                "ref",
                "set",
                invalid,
                "--revision",
                &revision,
                "--expected-generation",
                "0",
                "--request-key",
                "invalid-name",
            ],
        ] {
            let output = cli(project.path(), &args);
            assert!(!output.status.success(), "{args:?}");
            let error = String::from_utf8(output.stderr).unwrap();
            assert!(error.contains("reference name"), "{error:?}");
            assert!(
                !error.trim_end().chars().any(char::is_control) && !error.contains('\u{202e}'),
                "input controls in diagnostic: {error:?}"
            );
            assert_eq!(repo.status().unwrap(), selected);
            assert!(repo.references(None, 20).unwrap().is_empty());
            assert!(repo.receipt("invalid-name").unwrap().is_none());
        }
    }
}

#[test]
fn ref_mutation_requires_experimental_and_all_identity_arguments() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let selected = repo.commit("Author", "first", 0, "commit").unwrap().after;
    let revision = selected.revision.unwrap().to_string();
    let denied = cli(
        project.path(),
        &[
            "ref",
            "set",
            "topic",
            "--revision",
            &revision,
            "--expected-generation",
            "0",
            "--request-key",
            "denied",
        ],
    );
    assert!(!denied.status.success());
    assert!(
        String::from_utf8(denied.stderr)
            .unwrap()
            .contains("--experimental")
    );
    for args in [
        vec![
            "--experimental",
            "ref",
            "set",
            "topic",
            "--expected-generation",
            "0",
            "--request-key",
            "missing",
        ],
        vec![
            "--experimental",
            "ref",
            "set",
            "topic",
            "--revision",
            &revision,
            "--request-key",
            "missing",
        ],
        vec![
            "--experimental",
            "ref",
            "set",
            "topic",
            "--revision",
            &revision,
            "--expected-generation",
            "0",
        ],
    ] {
        assert!(
            !cli(project.path(), &args).status.success(),
            "accepted {args:?}"
        );
    }
    assert!(repo.receipt("denied").unwrap().is_none());
    assert!(repo.receipt("missing").unwrap().is_none());
    assert_eq!(repo.status().unwrap(), selected);
    assert!(
        text(success(project.path(), &["ref", "list"]))
            .trim()
            .is_empty()
    );
}

#[test]
fn cli_escapes_ref_names_and_request_keys_in_all_ref_and_receipt_output() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let revision = repo
        .commit("Author", "first", 0, "commit")
        .unwrap()
        .after
        .revision
        .unwrap()
        .to_string();
    let name = "topic/\"quoted\\name\u{202e}safe";
    let key = "request\n\r\t\u{1b}[31m\\\"";
    let accepted = success(
        project.path(),
        &[
            "--experimental",
            "ref",
            "set",
            name,
            "--revision",
            &revision,
            "--expected-generation",
            "0",
            "--request-key",
            key,
        ],
    );
    for output in [accepted.clone(), success(project.path(), &["receipt", key])] {
        let output = text(output);
        assert!(
            output.contains(&name.escape_debug().to_string()),
            "{output:?}"
        );
        assert!(
            output.contains(&key.escape_debug().to_string()),
            "{output:?}"
        );
        assert!(!output.contains('\u{1b}'));
        assert_eq!(
            output.lines().count(),
            1,
            "request keys cannot inject output lines"
        );
    }
    assert_eq!(
        accepted.stdout,
        success(project.path(), &["receipt", key]).stdout
    );
    for args in [vec!["ref", "show", name], vec!["ref", "list"]] {
        let output = text(success(project.path(), &args));
        assert!(
            output.contains(&name.escape_debug().to_string()),
            "{output:?}"
        );
        assert!(!output.contains(name), "unescaped name: {output:?}");
    }
}

#[test]
fn cli_ref_diagnostics_escape_reused_keys_and_stale_names() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let revision = repo
        .commit("Author", "first", 0, "commit")
        .unwrap()
        .after
        .revision
        .unwrap()
        .to_string();
    let key = "request\n\r\t\u{1b}[31m";
    let name = "topic/\u{202e}safe";
    success(
        project.path(),
        &[
            "--experimental",
            "ref",
            "set",
            name,
            "--revision",
            &revision,
            "--expected-generation",
            "0",
            "--request-key",
            key,
        ],
    );
    for args in [
        vec![
            "--experimental",
            "ref",
            "set",
            "different-name",
            "--revision",
            &revision,
            "--expected-generation",
            "0",
            "--request-key",
            key,
        ],
        vec![
            "--experimental",
            "commit",
            "--author",
            "Author",
            "-m",
            "different-kind",
            "--expected-generation",
            "1",
            "--request-key",
            key,
        ],
    ] {
        let output = cli(project.path(), &args);
        assert!(!output.status.success());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains(&key.escape_debug().to_string()), "{error:?}");
        assert!(!error.contains(key), "raw key in diagnostic: {error:?}");
        assert!(
            !error.contains(['\u{1b}', '\r', '\t']),
            "terminal controls in diagnostic: {error:?}"
        );
    }
    let stale = cli(
        project.path(),
        &[
            "--experimental",
            "ref",
            "set",
            name,
            "--revision",
            &revision,
            "--expected-generation",
            "0",
            "--request-key",
            "stale",
        ],
    );
    assert!(!stale.status.success());
    let error = String::from_utf8(stale.stderr).unwrap();
    assert!(
        error.contains(&name.escape_debug().to_string()),
        "{error:?}"
    );
    assert!(
        !error.contains(name),
        "raw bidi name in diagnostic: {error:?}"
    );
}

#[test]
fn cli_list_defaults_to_twenty_and_honors_binary_cursor_and_limit_bounds() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let revision = repo
        .commit("Author", "first", 0, "commit")
        .unwrap()
        .after
        .revision
        .unwrap()
        .to_string();
    for i in 0..23 {
        success(
            project.path(),
            &[
                "--experimental",
                "ref",
                "set",
                &format!("topic-{i:02}"),
                "--revision",
                &revision,
                "--expected-generation",
                "0",
                "--request-key",
                &format!("ref-{i}"),
            ],
        );
    }
    let default = text(success(project.path(), &["ref", "list"]));
    assert_eq!(
        default
            .lines()
            .filter(|line| line.contains("topic-"))
            .count(),
        20
    );
    assert!(default.contains("topic-00"));
    assert!(default.contains("topic-19"));
    assert!(!default.contains("topic-20"));
    let page = text(success(
        project.path(),
        &["ref", "list", "--after", "topic-19a", "--limit", "2"],
    ));
    assert!(page.contains("topic-20"));
    assert!(page.contains("topic-21"));
    assert!(!page.contains("topic-19"));
    assert!(!page.contains("topic-22"));
    assert!(
        text(success(project.path(), &["ref", "list", "--limit", "0"]))
            .trim()
            .is_empty()
    );
    let all = text(success(
        project.path(),
        &["ref", "list", "--limit", "100000"],
    ));
    assert_eq!(
        all.lines().filter(|line| line.contains("topic-")).count(),
        23
    );
    for limit in ["100001", "-1", "not-a-number"] {
        assert!(
            !cli(project.path(), &["ref", "list", "--limit", limit])
                .status
                .success()
        );
    }
}

#[test]
fn root_repository_option_scopes_ref_writes_and_receipt_lookup_not_current_directory() {
    let selected_project = tempfile::tempdir().unwrap();
    let decoy = tempfile::tempdir().unwrap();
    let mut selected_repo = Repository::init(selected_project.path()).unwrap();
    let decoy_repo = Repository::init(decoy.path()).unwrap();
    let revision = selected_repo
        .commit("Author", "first", 0, "commit")
        .unwrap()
        .after
        .revision
        .unwrap()
        .to_string();
    let output = Command::new(env!("CARGO_BIN_EXE_avc"))
        .current_dir(decoy.path())
        .arg("--repository")
        .arg(selected_project.path())
        .args([
            "--experimental",
            "ref",
            "set",
            "scoped",
            "--revision",
            &revision,
            "--expected-generation",
            "0",
            "--request-key",
            "scoped-key",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(selected_repo.receipt("scoped-key").unwrap().is_some());
    assert!(decoy_repo.receipt("scoped-key").unwrap().is_none());
    assert!(
        !cli(decoy.path(), &["receipt", "scoped-key"])
            .status
            .success()
    );
    assert_eq!(
        success(selected_project.path(), &["receipt", "scoped-key"]).stdout,
        output.stdout
    );
    assert_eq!(decoy_repo.status().unwrap().generation, 0);
    assert!(
        text(success(decoy.path(), &["ref", "list"]))
            .trim()
            .is_empty()
    );
}

#[test]
fn cli_ref_keys_cannot_be_reused_by_source_operations_or_undone() {
    let project = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(project.path()).unwrap();
    let selected = repo.commit("Author", "first", 0, "commit").unwrap().after;
    let revision = selected.revision.unwrap().to_string();
    let accepted = success(
        project.path(),
        &[
            "--experimental",
            "ref",
            "set",
            "topic",
            "--revision",
            &revision,
            "--expected-generation",
            "0",
            "--request-key",
            "ref-key",
        ],
    );
    for args in [
        vec![
            "--experimental",
            "commit",
            "--author",
            "Author",
            "-m",
            "no",
            "--expected-generation",
            "1",
            "--request-key",
            "ref-key",
        ],
        vec![
            "--experimental",
            "history",
            "undo",
            "ref-key",
            "--expected-generation",
            "1",
            "--request-key",
            "undo-ref",
        ],
        vec![
            "--experimental",
            "ref",
            "set",
            "topic",
            "--revision",
            &revision,
            "--expected-generation",
            "0",
            "--request-key",
            "commit",
        ],
        vec![
            "--experimental",
            "ref",
            "set",
            "../bad",
            "--revision",
            &revision,
            "--expected-generation",
            "0",
            "--request-key",
            "bad-name",
        ],
    ] {
        assert!(
            !cli(project.path(), &args).status.success(),
            "accepted {args:?}"
        );
    }
    assert!(repo.receipt("undo-ref").unwrap().is_none());
    assert!(repo.receipt("bad-name").unwrap().is_none());
    assert_eq!(repo.status().unwrap(), selected);
    assert_eq!(
        success(project.path(), &["receipt", "ref-key"]).stdout,
        accepted.stdout
    );
}
