use anyhow::{Context, Result};
pub use gix::Repository;

/// Open the repository at the current directory.
pub fn open_repo() -> Result<Repository> {
    gix::open(".").context("not a git repository (or any of the parent directories)")
}

/// Get the current HEAD commit ID as a hex string.
pub fn head_commit_id(repo: &Repository) -> Result<Option<String>> {
    let head = repo.head();
    match head {
        Ok(mut h) => {
            match h.peel_to_commit_in_place() {
                Ok(commit) => Ok(Some(commit.id().to_string())),
                Err(_) => Ok(None),
            }
        }
        Err(_) => Ok(None),
    }
}

/// Capture the working directory as a git tree (non-destructive).
/// Uses a temporary index file to avoid corrupting the user's staging area.
pub fn capture_workdir_tree(_repo: &Repository) -> Result<Option<String>> {
    let temp_index = std::env::temp_dir().join(format!("avc-index-{}", std::process::id()));

    // Use a temporary index file
    let result = std::process::Command::new("git")
        .args(["add", "-A"])
        .env("GIT_INDEX_FILE", &temp_index)
        .output()
        .context("failed to run git add")?;

    if !result.status.success() {
        let _ = std::fs::remove_file(&temp_index);
        let stderr = String::from_utf8_lossy(&result.stderr);
        anyhow::bail!("git add failed: {}", stderr);
    }

    // Write tree from the temp index
    let result = std::process::Command::new("git")
        .args(["write-tree"])
        .env("GIT_INDEX_FILE", &temp_index)
        .output()
        .context("failed to run git write-tree")?;

    // Clean up temp index
    let _ = std::fs::remove_file(&temp_index);

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        anyhow::bail!("git write-tree failed: {}", stderr);
    }

    let tree_sha = String::from_utf8_lossy(&result.stdout).trim().to_string();
    if tree_sha.is_empty() {
        return Ok(None);
    }

    Ok(Some(tree_sha))
}

/// Create a snapshot commit under a ref with the given tree and parent.
/// Returns the commit SHA.
pub fn create_snapshot_commit(
    repo: &Repository,
    ref_name: &str,
    tree_id: &str,
    parent: Option<&str>,
    message: &str,
) -> Result<String> {
    let tree_oid = gix::hash::ObjectId::from_hex(tree_id.as_bytes())
        .context("invalid tree SHA")?;

    let parents: Vec<gix::hash::ObjectId> = match parent {
        Some(p) => vec![gix::hash::ObjectId::from_hex(p.as_bytes())
            .context("invalid parent SHA")?],
        None => vec![],
    };

    let commit_id = repo
        .commit(ref_name, message, tree_oid, parents.iter().copied())
        .context("failed to create snapshot commit")?;

    Ok(commit_id.to_string())
}

/// Restore working dir + index to a commit's tree WITHOUT moving HEAD.
pub fn restore_workdir(commit_sha: &str, clean_untracked: bool) -> Result<()> {
    let repo = open_repo()?;
    let tree_sha = commit_tree_sha(&repo, commit_sha)?;

    // Get list of files in target tree FIRST (before we modify the index)
    let target_files: std::collections::HashSet<String> = {
        let output = std::process::Command::new("git")
            .args(["ls-tree", "-r", "--name-only", &tree_sha])
            .output()
            .context("failed to run git ls-tree")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("git ls-tree failed: {}", stderr);
        }

        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|s| s.to_string())
            .collect()
    };

    // Get list of files in working tree BEFORE restore (including untracked)
    let working_tree_files: Vec<String> = {
        // Use git ls-files for tracked files
        let tracked = std::process::Command::new("git")
            .args(["ls-files"])
            .output()
            .context("failed to run git ls-files")?;

        // Use git ls-files -o for other (untracked) files
        let untracked = std::process::Command::new("git")
            .args(["ls-files", "-o", "--exclude-standard"])
            .output()
            .context("failed to run git ls-files -o")?;

        let mut files = Vec::new();

        if tracked.status.success() {
            files.extend(
                String::from_utf8_lossy(&tracked.stdout)
                    .lines()
                    .map(|s| s.to_string()),
            );
        }

        if untracked.status.success() {
            files.extend(
                String::from_utf8_lossy(&untracked.stdout)
                    .lines()
                    .map(|s| s.to_string()),
            );
        }

        files
    };

    // Update index to match the tree (doesn't move HEAD)
    let output = std::process::Command::new("git")
        .args(["read-tree", &tree_sha])
        .output()
        .context("failed to run git read-tree")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git read-tree failed: {}", stderr);
    }

    // Update working tree to match the index
    let output = std::process::Command::new("git")
        .args(["checkout-index", "-a", "-f"])
        .output()
        .context("failed to run git checkout-index")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git checkout-index failed: {}", stderr);
    }

    // Remove files that were in working tree but not in target tree
    for file in &working_tree_files {
        if !target_files.contains(file) {
            let path = std::path::Path::new(file);
            if path.exists() {
                let _ = std::fs::remove_file(path);
                // Also try to remove empty parent directories
                if let Some(parent) = path.parent() {
                    if parent.exists() && parent.read_dir().map_or(false, |mut d| d.next().is_none()) {
                        let _ = std::fs::remove_dir(parent);
                    }
                }
            }
        }
    }

    if clean_untracked {
        let output = std::process::Command::new("git")
            .args(["clean", "-fd"])
            .output()
            .context("failed to run git clean")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("git clean failed: {}", stderr);
        }
    }

    Ok(())
}

/// Get the commit ID that a ref points to. Returns None if ref doesn't exist.
pub fn ref_commit_id(repo: &Repository, ref_path: &str) -> Result<Option<String>> {
    match repo.try_find_reference(ref_path)? {
        Some(mut reference) => {
            let peeled = reference
                .peel_to_commit()
                .context("ref does not point to a commit")?;
            Ok(Some(peeled.id().to_string()))
        }
        None => Ok(None),
    }
}

/// List all refs under a given prefix.
pub fn list_refs(repo: &Repository, prefix: &str) -> Result<Vec<String>> {
    let refs = repo.references().context("failed to access references")?;
    let prefixed = refs
        .prefixed(prefix.as_bytes())
        .context("failed to list prefixed references")?;
    let mut names = Vec::new();
    for reference in prefixed {
        let r = reference.map_err(|e| anyhow::anyhow!("failed to read reference: {}", e))?;
        let name = r.name();
        names.push(name.as_bstr().to_string());
    }
    Ok(names)
}

/// Delete a ref.
pub fn delete_ref(repo: &Repository, ref_path: &str) -> Result<()> {
    match repo.try_find_reference(ref_path)? {
        Some(reference) => {
            reference.delete().context("failed to delete ref")?;
            Ok(())
        }
        None => Ok(()),
    }
}

/// Get the tree SHA from a commit.
pub fn commit_tree_sha(repo: &Repository, commit_sha: &str) -> Result<String> {
    let oid = gix::hash::ObjectId::from_hex(commit_sha.as_bytes())
        .context("invalid commit SHA")?;
    let commit = repo.find_commit(oid).context("commit not found")?;
    let tree = commit.tree().context("failed to read tree")?;
    Ok(tree.id().to_string())
}

/// Check if a ref exists.
pub fn ref_exists(repo: &Repository, ref_path: &str) -> Result<bool> {
    Ok(ref_commit_id(repo, ref_path)?.is_some())
}

/// Get the current branch name, or "HEAD (detached)" if detached.
pub fn current_branch(repo: &Repository) -> Result<String> {
    let head = repo.head().context("failed to read HEAD")?;
    match head.referent_name() {
        Some(name) => {
            let name_str = name.as_bstr().to_string();
            Ok(name_str
                .strip_prefix("refs/heads/")
                .unwrap_or(&name_str)
                .to_string())
        }
        None => Ok("HEAD (detached)".to_string()),
    }
}

/// Get short SHA (first 8 chars) from a full SHA.
pub fn short_sha(sha: &str) -> &str {
    if sha.len() >= 8 { &sha[..8] } else { sha }
}
