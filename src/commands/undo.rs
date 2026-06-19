use anyhow::{Result, Context, bail};
use std::collections::HashSet;

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

    let entries = oplog::read_all(&root)?;

    // Walk backwards through the oplog, tracking which entries have been undone.
    // An entry is "undone" if there's an undo targeting it that hasn't been redone.
    let mut undone: HashSet<usize> = HashSet::new();
    let mut target: Option<(usize, oplog::OpEntry)> = None;

    for (i, entry) in entries.iter().enumerate().rev() {
        match entry {
            oplog::OpEntry::Redo { target_op, .. } => {
                // Redo re-activates the entry at target_op
                undone.remove(target_op);
            }
            oplog::OpEntry::Undo { target_op, .. } => {
                // Undo deactivates the entry at target_op
                undone.insert(*target_op);
            }
            _ if undone.contains(&i) => {
                // This entry has been undone — skip
                continue;
            }
            _ => {
                // First non-undone, non-undo/redo entry
                target = Some((i, entry.clone()));
                break;
            }
        }
    }

    let (op_index, last_op) = match target {
        Some((i, op)) => (i, op),
        None => {
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
        oplog::OpEntry::Auto { head, .. } | oplog::OpEntry::Run { head, .. } => {
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
        oplog::OpEntry::Undo { .. } | oplog::OpEntry::Redo { .. } => unreachable!(),
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
