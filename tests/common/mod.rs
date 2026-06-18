//! Test helpers for avc integration tests.
//!
//! Provides utilities for creating temporary git repos and running avc commands.

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

        std::process::Command::new("git")
            .args(["init"])
            .current_dir(path)
            .output()
            .expect("failed to init git repo");

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

        std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "initial"])
            .current_dir(path)
            .output()
            .expect("failed to create initial commit");

        let avc_bin = env!("CARGO_BIN_EXE_avc").to_string();

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

    /// Run an avc command and assert it succeeds. Returns stdout.
    pub fn run_success(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(output.status.success(),
            "avc {} failed: stderr={}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    /// Run an avc command and assert it succeeds. Returns stderr (where non-JSON output goes).
    pub fn run_success_stderr(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(output.status.success(),
            "avc {} failed: stderr={}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stderr).to_string()
    }

    /// Run an avc command and assert it succeeds. Returns (stdout, stderr).
    #[allow(dead_code)]
    pub fn run_success_both(&self, args: &[&str]) -> (String, String) {
        let output = self.run(args);
        assert!(output.status.success(),
            "avc {} failed: stderr={}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr));
        (
            String::from_utf8_lossy(&output.stdout).to_string(),
            String::from_utf8_lossy(&output.stderr).to_string(),
        )
    }

    /// Run an avc command and assert it fails. Returns stderr.
    #[allow(dead_code)]
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

    /// Get HEAD hash via git.
    pub fn head_hash(&self) -> String {
        let output = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(self.dir.path())
            .output()
            .expect("failed to get HEAD");
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    /// Run a git command in the repo.
    pub fn git(&self, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(self.dir.path())
            .output()
            .expect("failed to run git");
        assert!(output.status.success(),
            "git {} failed: stderr={}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).to_string()
    }
}
