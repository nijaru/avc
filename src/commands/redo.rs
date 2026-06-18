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

    // Read oplog and find the last undo entry
    let entries = oplog::read_all(&root)?;
    let last_undo = entries.iter().rev().find(|e| e.op == "undo");

    let last_undo = match last_undo {
        Some(op) => op.clone(),
        None => {
            if json {
                println!("{{\"status\": \"nothing_to_redo\"}}");
            } else {
                output::info("nothing to redo");
            }
            return Ok(());
        }
    };

    // The undo entry has `to` (where we restored to) and `target_op` (what was undone)
    let target_op_index = last_undo.target_op.context("undo entry missing target_op")?;
    let original_op = &entries[target_op_index];

    // Restore to the state after the original operation
    let restore_to = original_op.head.as_ref().context("original op missing head")?.clone();

    let current_head = git::head_hash()?.context("no HEAD")?;

    // Check if we're already at the restore point
    if current_head == restore_to {
        if json {
            println!("{{\"status\": \"nothing_to_redo\"}}");
        } else {
            output::info("nothing more to redo");
        }
        return Ok(());
    }

    // Hard reset to restore point
    git::reset_hard(&restore_to)?;

    // Log redo to oplog
    let entry = oplog::OpEntry::redo(&current_head, &restore_to, target_op_index);
    oplog::append(&root, &entry)?;

    if json {
        println!("{{\"status\": \"redone\", \"to\": \"{}\"}}", restore_to);
    } else {
        output::success(&format!("redone — restored to {}", output::hash(&restore_to)));
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
