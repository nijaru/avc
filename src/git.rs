use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;

/// Run a git command and return stdout as a String.
pub fn git(args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .output()
        .with_context(|| format!("failed to run git {:?}", args))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        if stderr.is_empty() {
            bail!("git {:?} failed (exit {})", args, output.status.code().unwrap_or(-1));
        } else {
            bail!("{}", stderr);
        }
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Run git, returning nothing on success.
pub fn git_void(args: &[&str]) -> Result<()> {
    git(args)?;
    Ok(())
}

/// Check if we're in a git repository.
pub fn is_git_repo() -> bool {
    Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Get the current branch name. Returns None if detached HEAD.
pub fn current_branch() -> Result<Option<String>> {
    let output = Command::new("git")
        .args(["symbolic-ref", "--short", "HEAD"])
        .output()?;

    if output.status.success() {
        Ok(Some(String::from_utf8_lossy(&output.stdout).trim().to_string()))
    } else {
        Ok(None)
    }
}

/// Get the short hash of HEAD. Returns None if no commits yet.
pub fn head_hash() -> Result<Option<String>> {
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()?;

    if output.status.success() {
        Ok(Some(String::from_utf8_lossy(&output.stdout).trim().to_string()))
    } else {
        Ok(None)
    }
}

/// Check if the working tree is dirty (staged + unstaged + untracked).
pub fn is_dirty() -> Result<bool> {
    // Check staged + unstaged
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .output()?;
    Ok(!status.stdout.is_empty())
}

/// Stage all changes (respecting .gitignore).
pub fn add_all() -> Result<()> {
    git_void(&["add", "-A"])
}

/// Create a commit with the given message. Returns the short hash.
pub fn commit(message: &str) -> Result<String> {
    git_void(&["commit", "-m", message, "--allow-empty"])?;
    head_hash()?.context("commit succeeded but no HEAD")
}

/// Create a commit with a message file. Returns the short hash.
pub fn commit_file(path: &Path) -> Result<String> {
    git_void(&["commit", "-F", &path.to_string_lossy(), "--allow-empty"])?;
    head_hash()?.context("commit succeeded but no HEAD")
}

/// Soft reset to a target, keeping changes staged.
pub fn reset_soft(hash: &str) -> Result<()> {
    git_void(&["reset", "--soft", hash])
}

/// Hard reset to a target, discarding all changes.
pub fn reset_hard(hash: &str) -> Result<()> {
    git_void(&["reset", "--hard", hash])
}

/// Get the full log output with a format string.
pub fn log_format(format: &str, limit: Option<usize>) -> Result<String> {
    let limit_str;
    let mut args = vec!["log", "--format", format];
    if let Some(n) = limit {
        limit_str = n.to_string();
        args.push("-n");
        args.push(&limit_str);
    }
    git(&args)
}

/// Get diff stat between two refs, or working tree if only one ref.
pub fn diff_stat(a: &str, b: Option<&str>) -> Result<String> {
    let mut args = vec!["diff", "--stat", a];
    if let Some(b) = b {
        args.push(b);
    }
    git(&args)
}

/// Get diff stat for staged changes.
pub fn diff_stat_staged() -> Result<String> {
    git(&["diff", "--cached", "--stat"])
}

/// Check if a ref exists.
pub fn ref_exists(hash: &str) -> bool {
    Command::new("git")
        .args(["rev-parse", "--verify", hash])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
