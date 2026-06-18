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

    // Auto-commit dirty working tree first
    track::auto_commit(&root, json)?;

    // Read oplog and find the last non-undo/redo operation
    let entries = oplog::read_all(&root)?;
    let (op_index, last_op) = entries
        .iter()
        .enumerate()
        .rev()
        .find(|(_, e)| e.op_type() != "undo" && e.op_type() != "redo")
        .map(|(i, e)| (i, e.clone()))
        .unzip();

    let (op_index, last_op) = match (op_index, last_op) {
        (Some(i), Some(op)) => (i, op),
        _ => {
            if json {
                println!("{{\"status\": \"nothing_to_undo\"}}");
            } else {
                output::info("nothing to undo");
            }
            return Ok(());
        }
    };

    // Get the state to restore to
    let restore_to = match &last_op {
        oplog::OpEntry::Init { .. } => {
            if json {
                println!("{{\"status\": \"nothing_to_undo\"}}");
            } else {
                output::info("nothing to undo — this is the beginning");
            }
            return Ok(());
        }
        oplog::OpEntry::Auto { head, .. } => {
            let parent = git::git(&["rev-parse", "--short", &format!("{}^", head)])?;
            parent.trim().to_string()
        }
        oplog::OpEntry::Save { squashed, head, .. }
        | oplog::OpEntry::Amend { squashed, head, .. } => {
            if let Some(last_auto) = squashed.last() {
                last_auto.clone()
            } else {
                let parent = git::git(&["rev-parse", "--short", &format!("{}^", head)])?;
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

    if current_head == restore_to {
        if json {
            println!("{{\"status\": \"nothing_to_undo\"}}");
        } else {
            output::info("nothing more to undo");
        }
        return Ok(());
    }

    git::reset_hard(&restore_to)?;

    let entry = oplog::OpEntry::undo(&current_head, &restore_to, op_index);
    oplog::append(&root, &entry)?;

    if json {
        println!("{{\"status\": \"undone\", \"to\": \"{}\"}}", restore_to);
    } else {
        output::success(&format!("undone — back to {}", output::hash(&restore_to)));
    }

    Ok(())
}
