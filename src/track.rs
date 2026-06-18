use anyhow::Result;
use std::path::Path;

use crate::git;
use crate::oplog;

/// Auto-commit dirty working tree if needed.
/// Returns the commit hash if a commit was made, None if tree was clean.
pub fn auto_commit(repo_root: &Path, json: bool) -> Result<Option<String>> {
    if !git::is_git_repo() {
        return Ok(None);
    }

    let head = git::head_hash()?;
    if head.is_none() {
        return Ok(None);
    }

    if !git::is_dirty()? {
        return Ok(None);
    }

    let status = git::porcelain_status()?;
    let files: Vec<String> = status
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.len() > 3 {
                Some(line[3..].to_string())
            } else {
                None
            }
        })
        .collect();

    if files.is_empty() {
        return Ok(None);
    }

    git::add_all()?;

    let file_list = if files.len() <= 3 {
        files.join(", ")
    } else {
        format!("{}, and {} more", files[..2].join(", "), files.len() - 2)
    };

    let message = format!("[avc:auto] {}", file_list);
    let hash = git::commit(&message)?;

    let branch = git::current_branch()?.unwrap_or_else(|| "HEAD".to_string());
    let entry = oplog::OpEntry::auto(&branch, &hash, files);
    oplog::append(repo_root, &entry)?;

    if !json {
        eprintln!("  \x1b[2mauto-saved {}\x1b[0m", hash);
    }

    Ok(Some(hash))
}
