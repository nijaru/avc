use anyhow::{Result, bail};

use crate::git;
use crate::oplog;
use crate::output;

/// Status is read-only — does NOT auto-commit.
pub fn run(json: bool) -> Result<()> {
    if !git::is_git_repo() {
        bail!("not a git repository");
    }

    let root = git::repo_root()?;
    let branch = git::current_branch()?.unwrap_or_else(|| "HEAD".to_string());
    let head = git::head_hash()?;

    let entries = oplog::read_all(&root)?;
    let last_save = entries.iter().rev().find(|e| e.op_type() == "save" || e.op_type() == "amend");

    let auto_since_save = count_auto_commits_since_last_save(&entries)?;
    let dirty = git::is_dirty()?;

    if json {
        let status = serde_json::json!({
            "branch": branch,
            "head": head,
            "last_save": last_save.map(|s| serde_json::json!({
                "commit": s.head(),
                "title": match &s {
                    oplog::OpEntry::Save { title, .. } | oplog::OpEntry::Amend { title, .. } => Some(title.as_str()),
                    _ => None,
                },
                "time": s.time(),
            })),
            "auto_commits_since_save": auto_since_save,
            "uncommitted_changes": dirty,
        });
        println!("{}", serde_json::to_string_pretty(&status)?);
        return Ok(());
    }

    output::label_value("branch", &output::branch(&branch));

    if let Some(save) = last_save {
        let commit = save.head().unwrap_or("?");
        let title = match save {
            oplog::OpEntry::Save { title, .. } | oplog::OpEntry::Amend { title, .. } => title.as_str(),
            _ => "(unknown)",
        };
        let time = output::time_ago(save.time());
        output::label_value("last save", &format!("{} {} — {}", output::hash(commit), title, time));
    } else {
        output::label_value("last save", "(none)");
    }

    if auto_since_save > 0 {
        output::label_value("auto-commits", &format!("{} since last save", auto_since_save));

        if let Some(save) = last_save {
            let save_hash = save.head().unwrap_or("HEAD");
            if let Ok(stat) = git::diff_stat(save_hash, None) {
                let stat = stat.trim();
                if !stat.is_empty() {
                    output::blank();
                    output::info("would save:");
                    for line in stat.lines().take(10) {
                        output::info(&format!("  {}", line));
                    }
                }
            }
        }
    } else {
        output::label_value("auto-commits", "0 (clean)");
    }

    if dirty {
        output::label_value("uncommitted", "yes");
    } else {
        output::label_value("uncommitted", "none");
    }

    Ok(())
}

fn count_auto_commits_since_last_save(entries: &[oplog::OpEntry]) -> Result<usize> {
    let mut count = 0;
    for entry in entries.iter().rev() {
        match entry.op_type() {
            "save" | "amend" => break,
            "auto" => count += 1,
            _ => {}
        }
    }
    Ok(count)
}
