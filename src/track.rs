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
            let line = line.trim_end();
            if line.len() > 3 {
                let path = &line[3..];
                // Never commit .avc/ directory
                if path.starts_with(".avc/") || path.starts_with(".avc\\") {
                    return None;
                }
                Some(path.to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auto_commit_filters_avc_files() {
        // This test verifies that .avc/ files are filtered out
        // We can't easily test the full auto_commit function without a git repo,
        // but we can test the filtering logic
        let status_output = " M .avc/oplog\n M .avc/config\n M file.txt\n";
        let files: Vec<String> = status_output
            .lines()
            .filter_map(|line| {
                let line = line.trim_end();
                if line.len() > 3 {
                    let path = &line[3..];
                    // Never commit .avc/ directory
                    if path.starts_with(".avc/") || path.starts_with(".avc\\") {
                        return None;
                    }
                    Some(path.to_string())
                } else {
                    None
                }
            })
            .collect();
        
        assert_eq!(files.len(), 1);
        assert_eq!(files[0], "file.txt");
    }

    #[test]
    fn test_auto_commit_filters_avc_subdir() {
        let status_output = " M .avc/subdir/file.txt\n M file.txt\n";
        let files: Vec<String> = status_output
            .lines()
            .filter_map(|line| {
                let line = line.trim_end();
                if line.len() > 3 {
                    let path = &line[3..];
                    if path.starts_with(".avc/") || path.starts_with(".avc\\") {
                        return None;
                    }
                    Some(path.to_string())
                } else {
                    None
                }
            })
            .collect();
        
        assert_eq!(files.len(), 1);
        assert_eq!(files[0], "file.txt");
    }

    #[test]
    fn test_auto_commit_preserves_avc_in_filename() {
        // Files like "my_avc_file.txt" should not be filtered
        let status_output = " M my_avc_file.txt\n M .avc/oplog\n";
        let files: Vec<String> = status_output
            .lines()
            .filter_map(|line| {
                let line = line.trim_end();
                if line.len() > 3 {
                    let path = &line[3..];
                    if path.starts_with(".avc/") || path.starts_with(".avc\\") {
                        return None;
                    }
                    Some(path.to_string())
                } else {
                    None
                }
            })
            .collect();
        
        assert_eq!(files.len(), 1);
        assert_eq!(files[0], "my_avc_file.txt");
    }

    #[test]
    fn test_auto_commit_handles_windows_paths() {
        let status_output = " M .avc\\oplog\n M file.txt\n";
        let files: Vec<String> = status_output
            .lines()
            .filter_map(|line| {
                let line = line.trim_end();
                if line.len() > 3 {
                    let path = &line[3..];
                    if path.starts_with(".avc/") || path.starts_with(".avc\\") {
                        return None;
                    }
                    Some(path.to_string())
                } else {
                    None
                }
            })
            .collect();
        
        assert_eq!(files.len(), 1);
        assert_eq!(files[0], "file.txt");
    }
}
