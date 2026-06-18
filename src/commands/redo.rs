use anyhow::{Result, Context, bail};

use crate::git;
use crate::oplog;
use crate::output;
use crate::track;

pub fn run(json: bool) -> Result<()> {
    if !git::is_git_repo() {
        bail!("not a git repository");
    }

    let root = git::repo_root()?;

    // Auto-commit current state (safety net)
    track::auto_commit(&root, json)?;

    // Read oplog and find the last undo entry
    let entries = oplog::read_all(&root)?;
    let last_undo = entries.iter().rev().find(|e| e.op_type() == "undo");

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

    let target_op_index = match &last_undo {
        oplog::OpEntry::Undo { target_op, .. } => *target_op,
        _ => unreachable!(),
    };

    let original_op = &entries[target_op_index];
    let restore_to = original_op.head().context("original op missing head")?.to_string();

    let current_head = git::head_hash()?.context("no HEAD")?;

    if current_head == restore_to {
        if json {
            println!("{{\"status\": \"nothing_to_redo\"}}");
        } else {
            output::info("nothing more to redo");
        }
        return Ok(());
    }

    git::reset_hard(&restore_to)?;

    let entry = oplog::OpEntry::redo(&current_head, &restore_to, target_op_index);
    oplog::append(&root, &entry)?;

    if json {
        println!("{{\"status\": \"redone\", \"to\": \"{}\"}}", restore_to);
    } else {
        output::success(&format!("redone — restored to {}", output::hash(&restore_to)));
    }

    Ok(())
}
