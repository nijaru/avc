use anyhow::{Result, bail};
use std::path::Path;

use crate::git;
use crate::oplog;
use crate::output;
use crate::track;

pub fn run(messages: Vec<String>, amend: bool, json: bool) -> Result<()> {
    if !git::is_git_repo() {
        bail!("not a git repository. Run `avc init` first.");
    }

    let root = repo_root()?;
    let branch = git::current_branch()?.unwrap_or_else(|| "HEAD".to_string());

    // Auto-commit dirty working tree first
    track::auto_commit(&root, json)?;

    if amend {
        do_amend(&root, &branch, &messages, json)
    } else {
        do_save(&root, &branch, &messages, json)
    }
}

fn do_save(root: &Path, branch: &str, messages: &[String], json: bool) -> Result<()> {
    // Find auto-commits since last non-auto commit
    let auto_commits = find_auto_commits_since_last_save()?;

    if auto_commits.is_empty() {
        if json {
            println!("{{\"status\": \"nothing_to_save\"}}");
        } else {
            output::info("nothing to save — no auto-commits since last save");
        }
        return Ok(());
    }

    // Build commit message
    let title = if messages.is_empty() {
        generate_message()?
    } else {
        messages.join("\n\n")
    };

    // Get the hash before first auto-commit (the parent of the first auto-commit)
    let parent = git::git(&["rev-parse", &format!("{}^", auto_commits.last().unwrap())])?;
    let parent = parent.trim();

    // Squash: soft reset to parent, then commit
    git::reset_soft(parent)?;
    git::commit(&title)?;

    let new_head = git::head_hash()?.context("commit succeeded but no HEAD")?;

    // Log to oplog
    let entry = oplog::OpEntry::save(branch, &new_head, &title, auto_commits.clone());
    oplog::append(root, &entry)?;

    if json {
        println!("{{\"status\": \"saved\", \"commit\": \"{}\", \"squashed\": {}}}",
            new_head, auto_commits.len());
    } else {
        output::success(&format!("saved {}", output::hash(&new_head)));
        output::label_value("title", &title);
        output::label_value("squashed", &format!("{} auto-commits", auto_commits.len()));
    }

    Ok(())
}

fn do_amend(root: &Path, branch: &str, messages: &[String], json: bool) -> Result<()> {
    // Find the last save commit
    let entries = oplog::read_all(root)?;
    let last_save = entries.iter().rev().find(|e| e.op == "save" || e.op == "amend");

    let save_commit = match last_save {
        Some(entry) => entry.commit.clone().unwrap_or_else(|| entry.head.clone().unwrap_or_default()),
        None => {
            if json {
                println!("{{\"status\": \"nothing_to_amend\"}}");
            } else {
                output::info("nothing to amend — no previous save found");
            }
            return Ok(());
        }
    };

    // Find auto-commits since that save
    let auto_commits = find_auto_commits_since(&save_commit)?;

    if auto_commits.is_empty() && messages.is_empty() {
        if json {
            println!("{{\"status\": \"nothing_to_amend\"}}");
        } else {
            output::info("nothing to amend — no changes since last save");
        }
        return Ok(());
    }

    // Get the message: use provided or keep original
    let title = if messages.is_empty() {
        // Keep original message
        let msg = git::git(&["log", "-1", "--format=%B", &save_commit])?;
        msg.trim().to_string()
    } else {
        messages.join("\n\n")
    };

    // Soft reset to the save commit's parent, then commit with new message
    let parent = git::git(&["rev-parse", &format!("{}^", save_commit)])?;
    let parent = parent.trim();

    git::reset_soft(parent)?;
    git::commit(&title)?;

    let new_head = git::head_hash()?.context("commit succeeded but no HEAD")?;

    // Log to oplog
    let entry = oplog::OpEntry::amend(branch, &new_head, &title, auto_commits.clone());
    oplog::append(root, &entry)?;

    if json {
        println!("{{\"status\": \"amended\", \"commit\": \"{}\"}}", new_head);
    } else {
        output::success(&format!("amended {}", output::hash(&new_head)));
        output::label_value("title", &title);
    }

    Ok(())
}

/// Find all [avc:auto] commits since the last non-auto commit.
fn find_auto_commits_since_last_save() -> Result<Vec<String>> {
    // Walk backwards from HEAD, collecting auto-commits until we hit a non-auto
    let mut auto_commits = Vec::new();
    let mut current = "HEAD".to_string();

    loop {
        let msg = git::git(&["log", "-1", "--format=%s", &current])?;
        let msg = msg.trim();

        if !msg.starts_with("[avc:auto]") {
            break;
        }

        let hash = git::git(&["rev-parse", "--short", &current])?;
        let hash = hash.trim().to_string();
        auto_commits.push(hash.clone());

        // Move to parent
        let parent = git::git(&["rev-parse", "--short", &format!("{}^", current)]);
        match parent {
            Ok(p) => current = p.trim().to_string(),
            Err(_) => break, // No parent (root commit)
        }
    }

    // Return in order (oldest first)
    auto_commits.reverse();
    Ok(auto_commits)
}

/// Find all [avc:auto] commits since a given commit.
fn find_auto_commits_since(since: &str) -> Result<Vec<String>> {
    let mut auto_commits = Vec::new();
    let mut current = "HEAD".to_string();

    loop {
        // Stop if we've reached the target commit
        let current_hash = git::git(&["rev-parse", "--short", &current])?;
        let current_hash = current_hash.trim();
        let target_hash = git::git(&["rev-parse", "--short", since])?;
        let target_hash = target_hash.trim();

        if current_hash == target_hash {
            break;
        }

        let msg = git::git(&["log", "-1", "--format=%s", &current])?;
        let msg = msg.trim();

        if msg.starts_with("[avc:auto]") {
            auto_commits.push(current_hash.to_string());
        }

        // Move to parent
        let parent = git::git(&["rev-parse", "--short", &format!("{}^", current)]);
        match parent {
            Ok(p) => current = p.trim().to_string(),
            Err(_) => break,
        }
    }

    auto_commits.reverse();
    Ok(auto_commits)
}

/// Generate a message from the files changed in auto-commits.
fn generate_message() -> Result<String> {
    // Get the diff stat for staged changes (after reset --soft)
    let stat = git::diff_stat_staged().unwrap_or_default();
    let stat = stat.trim();

    if stat.is_empty() {
        return Ok("Update files".to_string());
    }

    // Extract file names from the diff stat
    let files: Vec<&str> = stat
        .lines()
        .filter_map(|line| {
            // Format: " file.rs | 5 +++--"
            line.split('|').next().map(|f| f.trim())
        })
        .collect();

    let title = if files.is_empty() {
        "Update files".to_string()
    } else if files.len() <= 3 {
        format!("Update {}", files.join(", "))
    } else {
        format!("Update {}, and {} more", files[..2].join(", "), files.len() - 2)
    };

    let mut message = title;
    if !stat.is_empty() {
        message.push_str("\n\nFiles changed:\n");
        for line in stat.lines() {
            if let Some(file) = line.split('|').next() {
                let file = file.trim();
                if !file.is_empty() {
                    message.push_str(&format!("- {}\n", file));
                }
            }
        }
    }

    Ok(message)
}

fn repo_root() -> Result<std::path::PathBuf> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()?;
    if !output.status.success() {
        anyhow::bail!("not in a git repository");
    }
    Ok(std::path::PathBuf::from(
        String::from_utf8_lossy(&output.stdout).trim(),
    ))
}

use anyhow::Context;
