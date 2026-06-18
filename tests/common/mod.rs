//! Test helpers for avc integration tests.
//!
//! Provides utilities for creating temporary git repos and running avc commands.

use std::path::Path;
use tempfile::TempDir;

/// A temporary git repository with avc initialized.
pub struct TestRepo {
    pub dir: TempDir,
    pub avc_bin: String,
}

impl TestRepo {
    /// Create a new test repo with initial commit and avc initialized.
    pub fn new() -> Self {
        let dir = TempDir::new().expect("failed to create temp dir");
        let path = dir.path();

        // Initialize git repo
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(path)
            .output()
            .expect("failed to init git repo");

        // Configure git for testing
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(path)
            .output()
            .expect("failed to config git");

        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(path)
            .output()
            .expect("failed to config git");

        // Create initial commit (required for avc)
        std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "initial"])
            .current_dir(path)
            .output()
            .expect("failed to create initial commit");

        // Get avc binary path
        let avc_bin = env!("CARGO_BIN_EXE_avc").to_string();

        // Initialize avc
        let output = std::process::Command::new(&avc_bin)
            .args(["init"])
            .current_dir(path)
            .output()
            .expect("failed to run avc init");

        assert!(output.status.success(), "avc init failed: {}", 
            String::from_utf8_lossy(&output.stderr));

        TestRepo { dir, avc_bin }
    }

    /// Run an avc command and return the output.
    pub fn run(&self, args: &[&str]) -> std::process::Output {
        std::process::Command::new(&self.avc_bin)
            .args(args)
            .current_dir(self.dir.path())
            .output()
            .expect("failed to run avc")
    }

    /// Run an avc command and assert it succeeds.
    pub fn run_success(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(output.status.success(), 
            "avc {} failed: stderr={}", 
            args.join(" "), 
            String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    /// Run an avc command and assert it fails.
    pub fn run_failure(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(!output.status.success(), 
            "avc {} unexpectedly succeeded", 
            args.join(" "));
        String::from_utf8_lossy(&output.stderr).to_string()
    }

    /// Create a file in the repo.
    pub fn create_file(&self, name: &str, content: &str) {
        let path = self.dir.path().join(name);
        std::fs::write(path, content).expect("failed to create file");
    }

    /// Read a file from the repo.
    pub fn read_file(&self, name: &str) -> String {
        let path = self.dir.path().join(name);
        std::fs::read_to_string(path).expect("failed to read file")
    }

    /// Check if a file exists in the repo.
    pub fn file_exists(&self, name: &str) -> bool {
        self.dir.path().join(name).exists()
    }

    /// List staged files (via git diff --cached).
    pub fn staged_files(&self) -> Vec<String> {
        let output = std::process::Command::new("git")
            .args(["diff", "--cached", "--name-only"])
            .current_dir(self.dir.path())
            .output()
            .expect("failed to run git diff");

        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|s| s.to_string())
            .collect()
    }

    /// List files in git log (to verify refs don't leak).
    pub fn git_log_refs(&self) -> Vec<String> {
        let output = std::process::Command::new("git")
            .args(["log", "--oneline", "--all"])
            .current_dir(self.dir.path())
            .output()
            .expect("failed to run git log");

        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|s| s.to_string())
            .collect()
    }

    /// Get the list of refs (to verify hidden refs exist).
    pub fn git_refs(&self) -> Vec<String> {
        let output = std::process::Command::new("git")
            .args(["for-each-ref", "--format=%(refname)"])
            .current_dir(self.dir.path())
            .output()
            .expect("failed to run git for-each-ref");

        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|s| s.to_string())
            .collect()
    }
}
