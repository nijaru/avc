use anyhow::{Result, Context, bail};
use std::path::Path;

use crate::git;
use crate::oplog;
use crate::output;
use crate::track;

pub fn run(messages: Vec<String>, amend: bool, json: bool) -> Result<()> {
    if !git::is_git_repo() {
        bail!("not a git repository. Run `avc init` first.");
    }

    let root = git::repo_root()?;
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
    let auto_commits = find_auto_commits_since_last_save()?;

    if auto_commits.is_empty() {
        if json {
            println!("{{\"status\": \"nothing_to_save\"}}");
        } else {
            output::info("nothing to save — no auto-commits since last save");
        }
        return Ok(());
    }

    let title = if messages.is_empty() {
        generate_message()?
    } else {
        messages.join("\n\n")
    };

    let change_id = oplog::generate_change_id();

    // Build commit message with Change-Id trailer
    let commit_message = format!("{}\n\nChange-Id: {}", title, change_id);

    let parent = git::git(&["rev-parse", &format!("{}^", auto_commits.last().unwrap())])?;
    let parent = parent.trim();

    git::reset_soft(parent)?;
    git::commit(&commit_message)?;

    let new_head = git::head_hash()?.context("commit succeeded but no HEAD")?;

    let entry = oplog::OpEntry::save(branch, &new_head, &title, auto_commits.clone(), Some(change_id));
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
    let entries = oplog::read_all(root)?;
    let last_save = entries.iter().rev().find(|e| e.op_type() == "save" || e.op_type() == "amend");

    let (save_commit, existing_change_id) = match &last_save {
        Some(entry) => {
            let change_id = match entry {
                oplog::OpEntry::Save { change_id, .. } | oplog::OpEntry::Amend { change_id, .. } => change_id.clone(),
                _ => None,
            };
            (entry.head().unwrap_or_default().to_string(), change_id)
        }
        None => {
            if json {
                println!("{{\"status\": \"nothing_to_amend\"}}");
            } else {
                output::info("nothing to amend — no previous save found");
            }
            return Ok(());
        }
    };

    let auto_commits = find_auto_commits_since(&save_commit)?;

    if auto_commits.is_empty() && messages.is_empty() {
        if json {
            println!("{{\"status\": \"nothing_to_amend\"}}");
        } else {
            output::info("nothing to amend — no changes since last save");
        }
        return Ok(());
    }

    let title = if messages.is_empty() {
        let msg = git::git(&["log", "-1", "--format=%B", &save_commit])?;
        // Strip existing Change-Id trailer from message
        msg.trim()
            .lines()
            .take_while(|line| !line.starts_with("Change-Id:"))
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string()
    } else {
        messages.join("\n\n")
    };

    // Reuse existing Change-Id or generate new one
    let change_id = existing_change_id.unwrap_or_else(oplog::generate_change_id);

    // Build commit message with Change-Id trailer
    let commit_message = format!("{}\n\nChange-Id: {}", title, change_id);

    let parent = git::git(&["rev-parse", &format!("{}^", save_commit)])?;
    let parent = parent.trim();

    git::reset_soft(parent)?;
    git::commit(&commit_message)?;

    let new_head = git::head_hash()?.context("commit succeeded but no HEAD")?;

    let entry = oplog::OpEntry::amend(branch, &new_head, &title, auto_commits.clone(), Some(change_id));
    oplog::append(root, &entry)?;

    if json {
        println!("{{\"status\": \"amended\", \"commit\": \"{}\"}}", new_head);
    } else {
        output::success(&format!("amended {}", output::hash(&new_head)));
        output::label_value("title", &title);
    }

    Ok(())
}

fn find_auto_commits_since_last_save() -> Result<Vec<String>> {
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

        let parent = git::git(&["rev-parse", "--short", &format!("{}^", current)]);
        match parent {
            Ok(p) => current = p.trim().to_string(),
            Err(_) => break,
        }
    }

    auto_commits.reverse();
    Ok(auto_commits)
}

fn find_auto_commits_since(since: &str) -> Result<Vec<String>> {
    let mut auto_commits = Vec::new();
    let mut current = "HEAD".to_string();

    loop {
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

        let parent = git::git(&["rev-parse", "--short", &format!("{}^", current)]);
        match parent {
            Ok(p) => current = p.trim().to_string(),
            Err(_) => break,
        }
    }

    auto_commits.reverse();
    Ok(auto_commits)
}

fn generate_message() -> Result<String> {
    let stat = git::diff_stat_staged().unwrap_or_default();
    let stat = stat.trim();

    if stat.is_empty() {
        return Ok("Update files".to_string());
    }

    let files: Vec<&str> = stat
        .lines()
        .filter_map(|line| line.split('|').next().map(|f| f.trim()))
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
