use anyhow::{Context, Result, bail};
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

/// Get the repository root directory.
pub fn repo_root() -> Result<std::path::PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()?;
    if !output.status.success() {
        bail!("not in a git repository");
    }
    Ok(std::path::PathBuf::from(
        String::from_utf8_lossy(&output.stdout).trim(),
    ))
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

/// Soft reset to a target, keeping changes staged.
pub fn reset_soft(hash: &str) -> Result<()> {
    git_void(&["reset", "--soft", hash])
}

/// Hard reset to a target, discarding all changes.
pub fn reset_hard(hash: &str) -> Result<()> {
    git_void(&["reset", "--hard", hash])
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

/// Get the porcelain status output.
pub fn porcelain_status() -> Result<String> {
    git(&["status", "--porcelain"])
}
