//! A bounded identity/foreign-edit conformance case, not a Git adapter/benchmark.
use avc::{Id, Object};
use std::{
    collections::BTreeSet,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
fn git(dir: &Path, args: &[&str], input: Option<&[u8]>) -> Output {
    let mut command = Command::new("git");
    command
        .current_dir(dir)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "gc.auto=0",
            "-c",
            "protocol.file.allow=always",
        ])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "AVC fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "AVC fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid");
    for key in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
    ] {
        command.env_remove(key);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(input) = input {
        child.stdin.take().unwrap().write_all(input).unwrap();
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
fn text(output: Output) -> String {
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
fn present(dir: &Path) -> BTreeSet<String> {
    text(git(
        dir,
        &[
            "cat-file",
            "--batch-all-objects",
            "--batch-check=%(objectname)",
        ],
        None,
    ))
    .lines()
    .map(str::to_owned)
    .collect()
}
#[test]
fn filtered_git_edit_reuses_foreign_oid_without_claiming_native_source() {
    let temp = tempfile::tempdir().unwrap();
    let origin = temp.path().join("origin.git");
    git(
        temp.path(),
        &["init", "--bare", origin.to_str().unwrap()],
        None,
    );
    git(&origin, &["config", "uploadpack.allowFilter", "true"], None);
    let unknown_bytes = vec![b'z'; 128 * 1024];
    let known = text(git(
        &origin,
        &["hash-object", "-w", "--stdin"],
        Some(b"known\n"),
    ));
    let unknown = text(git(
        &origin,
        &["hash-object", "-w", "--stdin"],
        Some(&unknown_bytes),
    ));
    let tree = text(git(
        &origin,
        &["mktree"],
        Some(
            format!("100644 blob {known}\tedit.txt\n100644 blob {unknown}\tunseen.dat\n")
                .as_bytes(),
        ),
    ));
    let commit = text(git(
        &origin,
        &["commit-tree", &tree],
        Some(b"foreign root\n"),
    ));
    git(&origin, &["update-ref", "refs/heads/main", &commit], None);
    git(&origin, &["symbolic-ref", "HEAD", "refs/heads/main"], None);
    let thin = temp.path().join("thin");
    let url = format!("file://{}", origin.display());
    git(
        temp.path(),
        &[
            "clone",
            "--quiet",
            "--filter=blob:none",
            "--no-checkout",
            &url,
            thin.to_str().unwrap(),
        ],
        None,
    );
    assert!(!present(&thin).contains(&unknown));
    assert!(!present(&thin).contains(&known));
    let listing = text(git(&thin, &["ls-tree", "HEAD"], None));
    assert!(listing.contains(&unknown));
    assert!(!present(&thin).contains(&unknown));
    // An ordinary Git-relative tree edit can keep the unseen foreign commitment.
    let edited = text(git(
        &thin,
        &["hash-object", "-w", "--stdin"],
        Some(b"edited without checkout\n"),
    ));
    let changed = text(git(
        &thin,
        &["mktree", "--missing"],
        Some(
            format!("100644 blob {edited}\tedit.txt\n100644 blob {unknown}\tunseen.dat\n")
                .as_bytes(),
        ),
    ));
    let revision = text(git(
        &thin,
        &["commit-tree", &changed, "-p", &commit],
        Some(b"foreign conditional candidate\n"),
    ));
    assert!(!present(&thin).contains(&unknown));
    assert!(text(git(&thin, &["ls-tree", &revision], None)).contains(&unknown));
    let native = Object::Blob(unknown_bytes).id().unwrap();
    assert!(format!("avc1-k01-{unknown}").parse::<Id>().is_err());
    assert_ne!(native.to_string(), unknown);
    // No native root was emitted: foreign identity/availability stays foreign.
    assert!(!thin.join(".avc").exists());
    assert!(!thin.join("edit.txt").exists());
}
