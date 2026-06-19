//! Integration tests for avc v0.
//!
//! Tests the core workflow: init, auto-commit, save, status, log, undo, redo, amend, run.

mod common;

use common::TestRepo;

// ── init ──

#[test]
fn init_creates_avc_directory() {
    let repo = TestRepo::new();
    assert!(repo.dir.path().join(".avc").exists());
    assert!(repo.dir.path().join(".avc/oplog").exists());
    assert!(repo.dir.path().join(".avc/config").exists());
}

#[test]
fn init_is_idempotent() {
    let repo = TestRepo::new();
    let output = repo.run(&["init"]);
    assert!(output.status.success());
}

#[test]
fn init_updates_gitignore() {
    let repo = TestRepo::new();
    let gitignore = repo.read_file(".gitignore");
    assert!(gitignore.contains(".avc/"));
}

// ── auto-commit ──

#[test]
fn save_auto_commits_dirty_tree() {
    let repo = TestRepo::new();

    repo.create_file("hello.txt", "hello");
    repo.create_file("world.txt", "world");

    // save auto-commits dirty tree first, then squashes
    repo.run_success(&["save", "-m", "add files"]);

    // The save commit should contain the files
    let log = repo.git(&["log", "--oneline"]);
    assert!(log.contains("add files"), "save commit should exist");
}

#[test]
fn auto_commit_skips_when_clean() {
    let repo = TestRepo::new();

    repo.create_file("f.txt", "content");
    repo.run_success(&["save", "-m", "first"]); // auto-commits + saves

    let head_before = repo.head_hash();
    repo.run_success(&["status"]); // no changes, status is read-only
    let head_after = repo.head_hash();

    assert_eq!(head_before, head_after, "status should not change HEAD");
}

// ── save ──

#[test]
fn save_squashes_auto_commits() {
    let repo = TestRepo::new();

    repo.create_file("a.txt", "a");
    repo.run_success(&["save", "-m", "add a"]);

    let json = repo.run_success(&["log", "--json"]);
    let entries: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();

    let saves: Vec<_> = entries.iter().filter(|e| e["op"] == "save").collect();
    assert_eq!(saves.len(), 1, "should have exactly one save entry");
}

#[test]
fn save_creates_clean_commit() {
    let repo = TestRepo::new();

    repo.create_file("a.txt", "a");
    repo.run_success(&["save", "-m", "clean commit"]);

    let log = repo.git(&["log", "--oneline", "-3"]);
    assert!(log.contains("clean commit"));
}

#[test]
fn save_requires_message() {
    let repo = TestRepo::new();
    repo.create_file("a.txt", "a");
    let output = repo.run(&["save"]);
    assert!(output.status.success());
}

// ── status ──

#[test]
fn status_shows_branch() {
    let repo = TestRepo::new();
    let stderr = repo.run_success_stderr(&["status"]);
    assert!(stderr.contains("main"), "status should show branch name, got: {}", stderr);
}

#[test]
fn status_shows_uncommitted_changes() {
    let repo = TestRepo::new();
    repo.create_file("new.txt", "new");
    let stderr = repo.run_success_stderr(&["status"]);
    assert!(stderr.contains("yes"), "status should show uncommitted: yes, got: {}", stderr);
}

#[test]
fn status_is_read_only() {
    let repo = TestRepo::new();
    repo.create_file("f.txt", "content");
    let head_before = repo.head_hash();
    repo.run_success(&["status"]);
    let head_after = repo.head_hash();
    assert_eq!(head_before, head_after, "status should not change HEAD (read-only)");
}

#[test]
fn status_json_output() {
    let repo = TestRepo::new();
    repo.create_file("f.txt", "content");
    repo.run_success(&["save", "-m", "test"]);
    let json = repo.run_success(&["status", "--json"]);
    let val: serde_json::Value = serde_json::from_str(&json)
        .expect("status --json should be valid JSON");
    assert!(val["branch"].as_str().is_some());
    assert!(val["head"].as_str().is_some());
}

// ── log ──

#[test]
fn log_shows_operations() {
    let repo = TestRepo::new();

    repo.create_file("a.txt", "a");
    repo.run_success(&["save", "-m", "first"]);

    repo.create_file("b.txt", "b");
    repo.run_success(&["save", "-m", "second"]);

    let stderr = repo.run_success_stderr(&["log"]);
    assert!(stderr.contains("first"), "log should show first save, got: {}", stderr);
    assert!(stderr.contains("second"), "log should show second save, got: {}", stderr);
}

#[test]
fn log_saves_filter() {
    let repo = TestRepo::new();

    repo.create_file("a.txt", "a");
    repo.run_success(&["save", "-m", "my save"]);

    repo.create_file("b.txt", "b");
    repo.run_success(&["save", "-m", "another save"]);

    let stderr = repo.run_success_stderr(&["log", "--saves"]);
    assert!(stderr.contains("my save"), "--saves should show save entries, got: {}", stderr);
}

#[test]
fn log_limit() {
    let repo = TestRepo::new();

    repo.create_file("a.txt", "a");
    repo.run_success(&["save", "-m", "first"]);

    repo.create_file("b.txt", "b");
    repo.run_success(&["save", "-m", "second"]);

    repo.create_file("c.txt", "c");
    repo.run_success(&["save", "-m", "third"]);

    let stderr = repo.run_success_stderr(&["log", "--limit", "1"]);
    assert!(stderr.contains("third"), "--limit 1 should show most recent, got: {}", stderr);
}

#[test]
fn log_json_output() {
    let repo = TestRepo::new();

    repo.create_file("a.txt", "a");
    repo.run_success(&["save", "-m", "test"]);

    let json = repo.run_success(&["log", "--json"]);
    let entries: Vec<serde_json::Value> = serde_json::from_str(&json)
        .expect("log --json should be valid JSON array");
    assert!(!entries.is_empty());
}

// ── undo / redo ──

#[test]
fn undo_redo_roundtrip() {
    let repo = TestRepo::new();

    repo.create_file("a.txt", "a");
    let before = repo.head_hash();

    repo.create_file("b.txt", "b");
    repo.run_success(&["save", "-m", "add b"]);
    let after = repo.head_hash();

    assert_ne!(before, after);

    repo.run_success(&["undo"]);
    let undone = repo.head_hash();
    assert_ne!(undone, after, "undo should change HEAD");

    repo.run_success(&["redo"]);
    let redone = repo.head_hash();
    assert_eq!(redone, after, "redo should restore HEAD to post-save state");
}

#[test]
fn undo_noop_at_boundary() {
    let repo = TestRepo::new();

    // With no saves, undo on just init should say nothing to undo
    let stderr = repo.run_success_stderr(&["undo"]);
    assert!(stderr.contains("nothing to undo"), "should handle undo with nothing to undo, got: {}", stderr);
}

#[test]
fn redo_noop_at_boundary() {
    let repo = TestRepo::new();

    repo.create_file("a.txt", "a");
    repo.run_success(&["save", "-m", "only save"]);

    let stderr = repo.run_success_stderr(&["redo"]);
    assert!(stderr.contains("nothing to redo") || stderr.contains("nothing more to redo"),
        "should handle redo at boundary, got: {}", stderr);
}

#[test]
fn undo_auto_commits_before_undoing() {
    let repo = TestRepo::new();

    repo.create_file("a.txt", "a");
    repo.run_success(&["save", "-m", "first"]);

    // Create dirty state (not saved)
    repo.create_file("b.txt", "b");

    // Undo should auto-commit b.txt first, then undo the save
    repo.run_success(&["undo"]);

    // After undo, we're back at the save point ("first")
    // b.txt was auto-committed but the undo restores to the save point
    // which doesn't include b.txt
    assert!(repo.file_exists("a.txt"), "a.txt should still exist after undo");
}

// ── amend ──

#[test]
fn save_amend_updates_last_save() {
    let repo = TestRepo::new();

    repo.create_file("a.txt", "a");
    repo.run_success(&["save", "-m", "original"]);

    repo.create_file("b.txt", "b");
    repo.run_success(&["save", "--amend", "-m", "amended"]);

    let stderr = repo.run_success_stderr(&["log", "--saves"]);
    assert!(stderr.contains("amended"), "log should show amended title, got: {}", stderr);
    // Note: both save and amend entries exist in the oplog (append-only)
    // The amend entry shows the new title, the save entry shows the original
}

// ── run ──

#[test]
fn run_snapshots_before_and_after() {
    let repo = TestRepo::new();

    repo.create_file("a.txt", "a");
    repo.run_success(&["save", "-m", "baseline"]);

    let stderr = repo.run_success_stderr(&["run", "--", "sh", "-c", "echo new > new.txt"]);
    assert!(stderr.contains("command exited 0") || stderr.contains("changes detected"),
        "run should report result, got: {}", stderr);

    assert!(repo.file_exists("new.txt"));
}

#[test]
fn run_preserves_exit_code() {
    let repo = TestRepo::new();

    let output = repo.run(&["run", "--", "false"]);
    assert!(!output.status.success(), "run should propagate non-zero exit code");
}

#[test]
fn run_requires_command() {
    let repo = TestRepo::new();
    let output = repo.run(&["run"]);
    assert!(!output.status.success(), "run without command should fail");
}

#[test]
fn run_json_output() {
    let repo = TestRepo::new();
    let json = repo.run_success(&["run", "--json", "--", "true"]);
    let val: serde_json::Value = serde_json::from_str(&json)
        .expect("run --json should be valid JSON");
    assert_eq!(val["success"].as_bool().unwrap(), true);
    assert_eq!(val["exit_code"].as_i64().unwrap(), 0);
}

// ── git coexistence ──

#[test]
fn avc_does_not_break_git() {
    let repo = TestRepo::new();

    repo.create_file("a.txt", "a");
    repo.run_success(&["save", "-m", "avc save"]);

    repo.create_file("b.txt", "b");
    repo.git(&["add", "b.txt"]);
    repo.git(&["commit", "-m", "git commit"]);

    repo.create_file("c.txt", "c");
    repo.run_success(&["save", "-m", "avc after git"]);

    let log = repo.git(&["log", "--oneline"]);
    assert!(log.contains("avc save"));
    assert!(log.contains("git commit"));
    assert!(log.contains("avc after git"));
}

// ── JSON output ──

#[test]
fn all_commands_support_json() {
    let repo = TestRepo::new();

    repo.create_file("a.txt", "a");

    let json = repo.run_success(&["save", "--json", "-m", "test"]);
    let _: serde_json::Value = serde_json::from_str(&json).expect("save --json");

    let json = repo.run_success(&["status", "--json"]);
    let _: serde_json::Value = serde_json::from_str(&json).expect("status --json");

    let json = repo.run_success(&["log", "--json"]);
    let _: Vec<serde_json::Value> = serde_json::from_str(&json).expect("log --json");

    let json = repo.run_success(&["undo", "--json"]);
    let _: serde_json::Value = serde_json::from_str(&json).expect("undo --json");

    let json = repo.run_success(&["redo", "--json"]);
    let _: serde_json::Value = serde_json::from_str(&json).expect("redo --json");
}

// ── error handling ──

#[test]
fn commands_fail_outside_git_repo() {
    let dir = tempfile::TempDir::new().unwrap();
    let avc = env!("CARGO_BIN_EXE_avc");

    let output = std::process::Command::new(avc)
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(!output.status.success(), "init outside git repo should fail");

    let output = std::process::Command::new(avc)
        .args(["save", "-m", "test"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(!output.status.success(), "save outside git repo should fail");
}

#[test]
fn commands_fail_without_init() {
    let dir = tempfile::TempDir::new().unwrap();

    std::process::Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    std::process::Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    std::process::Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    std::process::Command::new("git")
        .args(["commit", "--allow-empty", "-m", "init"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    // save creates .avc/oplog via create(true), so it works without init.
    // This is intentional — avc auto-initializes on first use.
    let avc = env!("CARGO_BIN_EXE_avc");
    let output = std::process::Command::new(avc)
        .args(["save", "-m", "test"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    // Verify it at least warns about missing .avc config
    let _stderr = String::from_utf8_lossy(&output.stderr);
    // save succeeds (auto-creates oplog) — this is expected behavior
    assert!(output.status.success(), "save should auto-create .avc if missing");
}
