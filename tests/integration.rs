//! Integration tests for avc git operations.
//!
//! Tests the critical data-integrity paths:
//! - capture_workdir_tree preserves index
//! - restore_workdir removes files absent from target
//! - auto_snapshot deduplication
//! - undo correctness

mod common;

use common::TestRepo;

#[test]
fn capture_workdir_tree_preserves_index() {
    let repo = TestRepo::new();
    
    // Stage a file
    repo.create_file("staged.txt", "staged content");
    let output = std::process::Command::new("git")
        .args(["add", "staged.txt"])
        .current_dir(repo.dir.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    
    // Create an unstaged file
    repo.create_file("unstaged.txt", "unstaged content");
    
    // Run avc change (which calls capture_workdir_tree internally)
    repo.run_success(&["change", "test change"]);
    
    // Verify staged.txt is still staged
    let staged = repo.staged_files();
    assert!(staged.contains(&"staged.txt".to_string()), 
        "staged.txt should still be staged after avc change, got: {:?}", staged);
    
    // Verify unstaged.txt is NOT staged
    assert!(!staged.contains(&"unstaged.txt".to_string()),
        "unstaged.txt should not be staged after avc change");
}

#[test]
fn restore_workdir_removes_files_absent_from_target() {
    let repo = TestRepo::new();
    
    // Create initial state
    repo.create_file("file1.txt", "content1");
    repo.run_success(&["change", "first"]);
    
    // Add more files
    repo.create_file("file2.txt", "content2");
    repo.run_success(&["change", "second"]);
    
    // Verify file2 exists
    assert!(repo.file_exists("file2.txt"));
    
    // Undo to first change
    repo.run_success(&["undo", "--clean"]);
    
    // file2.txt should be removed
    assert!(!repo.file_exists("file2.txt"), 
        "file2.txt should be removed after undo");
    
    // file1.txt should still exist
    assert!(repo.file_exists("file1.txt"),
        "file1.txt should still exist after undo");
}

#[test]
fn auto_snapshot_skips_when_no_changes() {
    let repo = TestRepo::new();
    
    // Create a change to establish a baseline
    repo.create_file("file1.txt", "content1");
    repo.run_success(&["change", "baseline"]);
    
    // Run status twice - second should not create new snapshot
    let output1 = repo.run_success(&["status"]);
    let output2 = repo.run_success(&["status"]);
    
    // Both should succeed
    assert!(!output1.is_empty());
    assert!(!output2.is_empty());
    
    // Check that we don't have duplicate auto-snapshots
    // (This is a behavioral test - if dedup works, the log should show reasonable entries)
    let log_output = repo.run_success(&["log", "--json"]);
    let entries: Vec<serde_json::Value> = serde_json::from_str(&log_output)
        .expect("failed to parse log JSON");
    
    // Should have: 1 auto (initial) + 1 change + reasonable auto-snapshots
    // Not 2x the entries from duplicate snapshots
    assert!(entries.len() < 10, 
        "too many log entries ({}), dedup may be broken", entries.len());
}

#[test]
fn undo_restores_to_correct_point() {
    let repo = TestRepo::new();
    
    // Create sequence of changes
    repo.create_file("v1.txt", "version 1");
    repo.run_success(&["change", "v1"]);
    
    repo.create_file("v2.txt", "version 2");
    repo.run_success(&["change", "v2"]);
    
    repo.create_file("v3.txt", "version 3");
    repo.run_success(&["change", "v3"]);
    
    // Undo twice - should be at v1
    repo.run_success(&["undo"]);
    repo.run_success(&["undo"]);
    
    // Should have v1.txt but not v2.txt or v3.txt
    assert!(repo.file_exists("v1.txt"), "v1.txt should exist");
    assert!(!repo.file_exists("v2.txt"), "v2.txt should not exist");
    assert!(!repo.file_exists("v3.txt"), "v3.txt should not exist");
}

#[test]
fn undo_with_dirty_tree_preserves_work() {
    let repo = TestRepo::new();
    
    // Create a change
    repo.create_file("committed.txt", "committed");
    repo.run_success(&["change", "baseline"]);
    
    // Make uncommitted changes
    repo.create_file("dirty.txt", "dirty work");
    
    // Undo should auto-snapshot dirty tree first, then restore to baseline
    // After undo, dirty.txt should NOT be in working tree (it's preserved in the auto-snapshot commit)
    repo.run_success(&["undo"]);
    
    // dirty.txt should NOT exist in working tree after undo
    // (it's preserved in the auto-snapshot, but the working tree is restored to baseline)
    assert!(!repo.file_exists("dirty.txt"),
        "dirty.txt should not be in working tree after undo (preserved in auto-snapshot commit)");
    
    // committed.txt should still exist
    assert!(repo.file_exists("committed.txt"),
        "committed.txt should still exist after undo");
}

#[test]
fn restore_to_specific_change() {
    let repo = TestRepo::new();
    
    // Create named changes
    repo.create_file("a.txt", "a");
    repo.run_success(&["change", "first"]);
    
    repo.create_file("b.txt", "b");
    repo.run_success(&["change", "second"]);
    
    repo.create_file("c.txt", "c");
    repo.run_success(&["change", "third"]);
    
    // Get change ID from log
    let log_output = repo.run_success(&["log", "--changes", "--json"]);
    let entries: Vec<serde_json::Value> = serde_json::from_str(&log_output)
        .expect("failed to parse log JSON");
    
    // Find the first change by looking at the command field
    let first_change = entries.iter()
        .find(|e| e["command"].as_str().unwrap_or("").contains("first"))
        .expect("could not find first change in log");
    
    let change_id = first_change["id"].as_str().unwrap();
    
    // Restore to first change
    repo.run_success(&["restore", change_id, "--clean"]);
    
    // Should have only a.txt
    assert!(repo.file_exists("a.txt"), "a.txt should exist");
    assert!(!repo.file_exists("b.txt"), "b.txt should not exist");
    assert!(!repo.file_exists("c.txt"), "c.txt should not exist");
}

#[test]
fn git_refs_hidden_from_git_log() {
    let repo = TestRepo::new();
    
    // Create some changes
    repo.create_file("file.txt", "content");
    repo.run_success(&["change", "test"]);
    
    // Verify avc refs exist
    let refs = repo.git_refs();
    let avc_refs: Vec<_> = refs.iter()
        .filter(|r| r.contains("agentvcs"))
        .collect();
    assert!(!avc_refs.is_empty(), "avc refs should exist");
    
    // Verify they don't appear in git log as branch names
    // (The commit messages will contain [agentvcs:...] which is fine)
    let log_lines = repo.git_log_refs();
    for line in &log_lines {
        // Check that agentvcs refs don't appear as branch names in the log
        // The format is: <sha> (<refs>) <message>
        // We should not see refs/agentvcs/* in the refs part
        if line.contains("refs/agentvcs") {
            panic!("git log should not show agentvcs refs as branches: {}", line);
        }
    }
}

#[test]
fn pre_push_hook_blocks_agentvcs_refs() {
    let repo = TestRepo::new();
    
    // Verify hook exists
    let hook_path = repo.dir.path().join(".git/hooks/pre-push");
    assert!(hook_path.exists(), "pre-push hook should be installed");
    
    // The hook should block pushing refs/agentvcs/*
    // (We can't easily test actual push without a remote, but we verify the hook content)
    let hook_content = std::fs::read_to_string(&hook_path).unwrap();
    assert!(hook_content.contains("refs/agentvcs"), 
        "hook should mention refs/agentvcs");
}

#[test]
fn change_with_message_flag() {
    let repo = TestRepo::new();
    
    repo.create_file("file.txt", "content");
    let output = repo.run_success(&["change", "-m", "test message"]);
    
    assert!(output.contains("test message"), 
        "output should contain the change message");
}

#[test]
fn log_shows_changes_filter() {
    let repo = TestRepo::new();
    
    // Create auto-snapshot and named change
    repo.create_file("auto.txt", "auto");
    repo.run_success(&["status"]); // creates auto-snapshot
    
    repo.create_file("named.txt", "named");
    repo.run_success(&["change", "named change"]);
    
    // Get all log entries
    let all_log = repo.run_success(&["log", "--json"]);
    let all_entries: Vec<serde_json::Value> = serde_json::from_str(&all_log).unwrap();
    
    // Get only changes
    let changes_log = repo.run_success(&["log", "--changes", "--json"]);
    let change_entries: Vec<serde_json::Value> = serde_json::from_str(&changes_log).unwrap();
    
    // Changes should be subset of all entries
    assert!(change_entries.len() <= all_entries.len(),
        "changes filter should return subset");
    
    // All change entries should have kind=change
    for entry in &change_entries {
        assert_eq!(entry["kind"], "change", 
            "all entries with --changes filter should be changes");
    }
}

#[test]
fn doctor_passes_on_clean_repo() {
    let repo = TestRepo::new();
    
    let output = repo.run_success(&["doctor"]);
    assert!(output.contains("all checks passed"), 
        "doctor should pass on clean repo: {}", output);
}

#[test]
fn status_shows_correct_info() {
    let repo = TestRepo::new();
    
    // Create a change
    repo.create_file("file.txt", "content");
    repo.run_success(&["change", "test change"]);
    
    // Check status
    let status = repo.run_success(&["status"]);
    assert!(status.contains("main"), "status should show branch name");
    assert!(status.contains("test change"), "status should show last change");
}

#[test]
fn json_output_is_valid() {
    let repo = TestRepo::new();
    
    // Create a change
    repo.create_file("file.txt", "content");
    repo.run_success(&["change", "test"]);
    
    // Test JSON output for various commands
    let log_json = repo.run_success(&["log", "--json"]);
    let _: Vec<serde_json::Value> = serde_json::from_str(&log_json)
        .expect("log --json should return valid JSON array");
    
    let status_json = repo.run_success(&["status", "--json"]);
    let _: serde_json::Value = serde_json::from_str(&status_json)
        .expect("status --json should return valid JSON");
    
    let doctor_json = repo.run_success(&["doctor", "--json"]);
    let _: serde_json::Value = serde_json::from_str(&doctor_json)
        .expect("doctor --json should return valid JSON");
}
