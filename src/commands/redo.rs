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

    // Auto-commit current state (safety net)
    track::auto_commit(&root, json)?;

    let entries = oplog::read_all(&root)?;

    // Walk backwards to find the last undo that hasn't been redone.
    // Track which undos have been redone.
    let mut redone_undos: HashSet<usize> = HashSet::new();
    let mut target: Option<(usize, usize)> = None; // (undo_index, target_op_index)

    for (i, entry) in entries.iter().enumerate().rev() {
        match entry {
            oplog::OpEntry::Redo { target_op, .. } => {
                // This redo re-activates target_op. The undo that targeted
                // target_op is now effectively redone.
                // Find the most recent undo targeting target_op and mark it redone.
                for j in (0..i).rev() {
                    if let oplog::OpEntry::Undo { target_op: undo_target, .. } = &entries[j] {
                        if *undo_target == *target_op {
                            redone_undos.insert(j);
                            break;
                        }
                    }
                }
            }
            oplog::OpEntry::Undo { target_op, .. } if !redone_undos.contains(&i) => {
                // Found an undo that hasn't been redone
                target = Some((i, *target_op));
                break;
            }
            _ => {}
        }
    }

    let (undo_index, target_op_index) = match target {
        Some(t) => t,
        None => {
            if json {
                println!("{{\"status\": \"nothing_to_redo\"}}");
            } else {
                output::info("nothing to redo");
            }
            return Ok(());
        }
    };

    // Check if there are any new operations after the undo
    // If so, we can't redo because it would overwrite those changes
    let has_ops_after_undo = entries.iter().skip(undo_index + 1)
        .any(|e| e.op_type() != "undo" && e.op_type() != "redo");

    if has_ops_after_undo {
        if json {
            println!("{{\"status\": \"cannot_redo\", \"reason\": \"new operations after undo\"}}");
        } else {
            output::info("cannot redo — new operations were performed after the undo");
        }
        return Ok(());
    }

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
