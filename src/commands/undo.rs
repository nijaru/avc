use anyhow::{Context, Result};

use crate::{db, git, id};

pub fn run(clean: bool, json: bool) -> Result<()> {
    let conn = db::open(&db::db_path()?)?;

    // Auto-snapshot first (non-destructive)
    let _ = crate::snapshot::auto_snapshot(std::path::Path::new("."), &conn)?;

    // Get all operations
    let all_ops = db::list_operations(&conn, 100)?;

    // Find the most recent non-auto operation (this is what we're undoing)
    let most_recent_non_auto = all_ops.iter().find(|op| op.kind != "auto");

    let most_recent_op = match most_recent_non_auto {
        Some(op) => op,
        None => {
            if json {
                println!("{{\"error\": \"nothing to undo\"}}");
            } else {
                println!("Nothing to undo.");
            }
            return Ok(());
        }
    };

    // If the most recent non-auto is init, nothing to undo
    if most_recent_op.kind == "init" {
        if json {
            println!("{{\"error\": \"nothing to undo\"}}");
        } else {
            println!("Nothing to undo.");
        }
        return Ok(());
    }

    // Find the target state to restore to
    let target_ref = find_undo_target(&all_ops, most_recent_op)?;

    let target_ref = match target_ref {
        Some(r) => r,
        None => {
            if json {
                println!("{{\"error\": \"cannot determine previous state\"}}");
            } else {
                println!("Cannot determine previous state.");
            }
            return Ok(());
        }
    };

    // Get the commit SHA from the ref
    let repo = git::open_repo()?;
    let commit_sha = git::ref_commit_id(&repo, &target_ref)?
        .context("ref does not point to a commit")?;

    // Restore working dir + index WITHOUT moving HEAD
    git::restore_workdir(&commit_sha, clean)?;

    // Record operation
    let before_commit = git::head_commit_id(&repo)?.unwrap_or_default();
    let op_id = id::new_op_id();
    db::insert_operation(
        &conn,
        &op_id,
        "cli",
        Some("undo"),
        "undo",
        Some(&before_commit),
        Some(&target_ref),
    )?;

    if json {
        println!("{{\"status\": \"restored\", \"target_ref\": \"{}\"}}", target_ref);
    } else {
        println!("Restored to {}", target_ref);
    }

    Ok(())
}

/// Find the target state to restore to when undoing an operation.
fn find_undo_target(
    all_ops: &[db::Operation],
    target_op: &db::Operation,
) -> Result<Option<String>> {
    // Find the index of the target operation
    let target_idx = match all_ops.iter().position(|op| op.id == target_op.id) {
        Some(idx) => idx,
        None => return Ok(None),
    };

    // Case 1: Undoing a change operation
    // Restore to the most recent change BEFORE this one
    if target_op.kind == "change" {
        for i in (target_idx + 1)..all_ops.len() {
            if all_ops[i].kind == "change" {
                return Ok(all_ops[i].after_ref.clone());
            }
        }
        // No previous change found - look for the auto-snapshot before this change
        // This handles the case of undoing the first change
        for i in (target_idx + 1)..all_ops.len() {
            if all_ops[i].kind == "auto" {
                return Ok(all_ops[i].after_ref.clone());
            }
        }
        return Ok(None);
    }

    // Case 2: Undoing an undo or restore operation
    // The undo/restore has an after_ref pointing to what it restored TO
    // We want to restore to the state BEFORE that target
    // (i.e., the most recent change before the undo's target)
    if target_op.kind == "undo" || target_op.kind == "restore" {
        // Get the target of the undo we're undoing
        if let Some(ref undo_target) = target_op.after_ref {
            // Find the operation that has this as its after_ref
            // This is the change that the undo was targeting
            let target_change_idx = all_ops.iter().position(|op| {
                op.after_ref.as_ref() == Some(undo_target) && op.kind == "change"
            });

            if let Some(change_idx) = target_change_idx {
                // Now find the most recent change BEFORE that change
                for i in (change_idx + 1)..all_ops.len() {
                    if all_ops[i].kind == "change" {
                        return Ok(all_ops[i].after_ref.clone());
                    }
                }
            }
        }
    }

    Ok(None)
}
