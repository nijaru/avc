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
    let last_undo_index = entries.iter().rposition(|e| e.op_type() == "undo");

    let (last_undo_index, last_undo) = match last_undo_index {
        Some(i) => (i, entries[i].clone()),
        None => {
            if json {
                println!("{{\"status\": \"nothing_to_redo\"}}");
            } else {
                output::info("nothing to redo");
            }
            return Ok(());
        }
    };

    // Check if there are any operations after the last undo
    // If so, we can't redo because it would overwrite those changes
    let has_ops_after_undo = entries.iter().skip(last_undo_index + 1)
        .any(|e| e.op_type() != "undo" && e.op_type() != "redo");

    if has_ops_after_undo {
        if json {
            println!("{{\"status\": \"cannot_redo\", \"reason\": \"new operations after undo\"}}");
        } else {
            output::info("cannot redo — new operations were performed after the undo");
        }
        return Ok(());
    }

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
