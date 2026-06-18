use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::Connection;
use std::path::Path;

use crate::{db, git, id};

const DEFAULT_MAX_AUTO_SNAPSHOTS: u32 = 100;

/// Auto-snapshot the working directory if it has changed since the last snapshot.
/// Returns Some((op_id, ref_path)) if a snapshot was created, None if skipped.
pub fn auto_snapshot(_repo_path: &Path, conn: &Connection) -> Result<Option<(String, String)>> {
    let repo = git::open_repo()?;

    // Check if there's a previous snapshot to compare against
    let latest_op = db::get_latest_ref_operation(conn, "")?;

    // Mtime check: scan working dir for changes since last snapshot
    if let Some(ref op) = latest_op {
        // Get the creation time of the latest snapshot
        let snapshot_time = chrono::DateTime::parse_from_rfc3339(&op.created_at)
            .context("invalid snapshot timestamp")?
            .with_timezone(&Utc);

        // Check if any tracked files have been modified since then
        if !working_dir_changed(snapshot_time)? {
            return Ok(None);
        }
    }

    // Capture working directory tree (non-destructive)
    let tree_sha = match git::capture_workdir_tree(&repo)? {
        Some(sha) => sha,
        None => return Ok(None), // No changes to snapshot
    };

    // Determine parent: last timeline point's commit, or HEAD, or none (empty repo)
    let parent = if let Some(ref op) = latest_op {
        // Get the commit SHA from the latest operation's after_ref
        if let Some(ref after_ref) = op.after_ref {
            git::ref_commit_id(&repo, after_ref)?.or_else(|| {
                // after_ref might be a direct SHA (for changes without refs)
                Some(after_ref.clone())
            })
        } else {
            // Try HEAD as fallback
            git::head_commit_id(&repo)?
        }
    } else {
        // No previous operations - use HEAD or none (empty repo)
        git::head_commit_id(&repo)?
    };

    // Create snapshot
    let op_id = id::new_op_id();
    let ref_path = format!("refs/agentvcs/auto/{}", op_id);
    let message = "[agentvcs:snapshot] auto";

    let _commit_sha = git::create_snapshot_commit(
        &repo,
        &ref_path,
        &tree_sha,
        parent.as_deref(),
        message,
    )?;

    // Record operation
    let before_commit = git::head_commit_id(&repo)?.unwrap_or_default();
    db::insert_operation(
        conn,
        &op_id,
        "cli",
        Some("auto"),
        "auto",
        Some(&before_commit),
        Some(&ref_path),
    )?;

    // Prune old auto-snapshots if needed
    prune_auto_snapshots(conn, &repo)?;

    Ok(Some((op_id, ref_path)))
}

/// Check if any files in the working directory have been modified since the given time.
fn working_dir_changed(_since: chrono::DateTime<Utc>) -> Result<bool> {
    // Use git status --porcelain to check for any uncommitted changes
    // This is more reliable than mtime checking
    let output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .context("failed to run git status")?;

    if !output.status.success() {
        // If git status fails, assume changed to be safe
        return Ok(true);
    }

    let status = String::from_utf8_lossy(&output.stdout);
    if !status.trim().is_empty() {
        return Ok(true);
    }

    Ok(false)
}

/// Prune oldest auto-snapshots if count exceeds limit.
fn prune_auto_snapshots(conn: &Connection, repo: &git::Repository) -> Result<()> {
    let count = db::count_auto_snapshots(conn)?;
    let max = DEFAULT_MAX_AUTO_SNAPSHOTS;

    if count <= max {
        return Ok(());
    }

    let to_prune = count - max;
    for _ in 0..to_prune {
        if let Some(ref_path) = db::get_oldest_auto_ref(conn)? {
            // Delete the ref
            git::delete_ref(repo, &ref_path)?;

            // We don't delete the DB record - it stays for timeline history.
            // The commit object will be GC'd by git gc eventually.
        }
    }

    Ok(())
}

/// Create a named change (non-auto snapshot with a title).
pub fn create_change(
    conn: &Connection,
    title: &str,
) -> Result<(String, String, String)> {
    let repo = git::open_repo()?;

    // Capture working directory tree
    let tree_sha = match git::capture_workdir_tree(&repo)? {
        Some(sha) => sha,
        None => {
            // No changes - use HEAD's tree
            let head_commit = git::head_commit_id(&repo)?
                .context("no HEAD commit and no working dir changes")?;
            let repo2 = git::open_repo()?;
            git::commit_tree_sha(&repo2, &head_commit)?
        }
    };

    // Parent = HEAD (changes are children of HEAD, not previous snapshot)
    let parent = git::head_commit_id(&repo)?;

    // Create change
    let change_id = id::new_change_id();
    let ref_path = format!("refs/agentvcs/changes/{}", change_id);
    let message = format!("[agentvcs:change] {}", title);

    let commit_sha = git::create_snapshot_commit(
        &repo,
        &ref_path,
        &tree_sha,
        parent.as_deref(),
        &message,
    )?;

    // Record in DB
    db::insert_change(conn, &change_id, title, &commit_sha)?;

    // Record operation
    let before_commit = git::head_commit_id(&repo)?.unwrap_or_default();
    let op_id = id::new_op_id();
    db::insert_operation(
        conn,
        &op_id,
        "cli",
        Some(&format!("change {}", title)),
        "change",
        Some(&before_commit),
        Some(&ref_path),
    )?;

    Ok((change_id, ref_path, commit_sha))
}
