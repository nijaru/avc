use anyhow::{Result, bail};

use crate::git;
use crate::oplog;
use crate::output;
use crate::track;

pub fn run(json: bool) -> Result<()> {
    if !git::is_git_repo() {
        bail!("not a git repository");
    }

    let root = repo_root()?;

    // Auto-commit current state (safety net)
    track::auto_commit(&root, json)?;

    // Read oplog and find the last non-undo/redo operation
    let entries = oplog::read_all(&root)?;
    let last_op = entries.iter().rev().find(|e| {
        e.op != "undo" && e.op != "redo"
    });

    let last_op = match last_op {
        Some(op) => op.clone(),
        None => {
            if json {
                println!("{{\"status\": \"nothing_to_undo\"}}");
            } else {
                output::info("nothing to undo");
            }
            return Ok(());
        }
    };

    let op_index = entries.iter().rposition(|e| e.op == last_op.op && e.head == last_op.head && e.time == last_op.time)
        .context("could not find operation in oplog")?;

    // Get the state to restore to
    let restore_to = match last_op.op.as_str() {
        "init" => {
            // Undo init: just a message, can't really undo
            if json {
                println!("{{\"status\": \"nothing_to_undo\"}}");
            } else {
                output::info("nothing to undo — this is the beginning");
            }
            return Ok(());
        }
        "auto" => {
            // Undo an auto-commit: restore to the commit before this one
            let head = last_op.head.as_ref().context("auto op missing head")?;
            let parent = git::git(&["rev-parse", "--short", &format!("{}^", head)])?;
            parent.trim().to_string()
        }
        "save" | "amend" => {
            // Undo a save: restore to the state before the save
            // The squashed auto-commits are in the oplog, restore to the last one
            if let Some(squashed) = &last_op.squashed {
                if let Some(last_auto) = squashed.last() {
                    last_auto.clone()
                } else {
                    // No squashed commits, restore to parent of save
                    let commit = last_op.commit.as_ref().or(last_op.head.as_ref()).context("save op missing commit")?;
                    let parent = git::git(&["rev-parse", "--short", &format!("{}^", commit)])?;
                    parent.trim().to_string()
                }
            } else {
                let commit = last_op.commit.as_ref().or(last_op.head.as_ref()).context("save op missing commit")?;
                let parent = git::git(&["rev-parse", "--short", &format!("{}^", commit)])?;
                parent.trim().to_string()
            }
        }
        _ => {
            if json {
                println!("{{\"status\": \"nothing_to_undo\"}}");
            } else {
                output::info("nothing to undo");
            }
            return Ok(());
        }
    };

    let current_head = git::head_hash()?.context("no HEAD")?;

    // Check if we're already at the restore point
    if current_head == restore_to {
        if json {
            println!("{{\"status\": \"nothing_to_undo\"}}");
        } else {
            output::info("nothing more to undo");
        }
        return Ok(());
    }

    // Hard reset to restore point
    git::reset_hard(&restore_to)?;

    // Log undo to oplog
    let entry = oplog::OpEntry::undo(&current_head, &restore_to, op_index);
    oplog::append(&root, &entry)?;

    if json {
        println!("{{\"status\": \"undone\", \"to\": \"{}\"}}", restore_to);
    } else {
        output::success(&format!("undone — back to {}", output::hash(&restore_to)));
    }

    Ok(())
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
