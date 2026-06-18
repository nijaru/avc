use anyhow::{Context, Result};

use crate::{db, git, id};

pub fn run(target_id: &str, clean: bool, json: bool) -> Result<()> {
    let conn = db::open(&db::db_path()?)?;

    // Auto-snapshot first (non-destructive)
    let _ = crate::snapshot::auto_snapshot(std::path::Path::new("."), &conn);

    // Find target operation by prefix match
    let target_op = db::get_operation_by_prefix(&conn, target_id)?;

    // Also try change id prefix
    let target_change = db::get_change_by_prefix(&conn, target_id)?;

    // Determine which target to use
    let (target_ref, _target_kind) = if let Some(ref op) = target_op {
        let ref_path = op.after_ref.as_ref()
            .context("operation has no associated ref")?;
        (ref_path.clone(), op.kind.clone())
    } else if let Some(ref change) = target_change {
        let ref_path = format!("refs/agentvcs/changes/{}", change.id);
        (ref_path, "change".to_string())
    } else {
        if json {
            println!("{{\"error\": \"no timeline point matching '{}'\"}}", target_id);
        } else {
            eprintln!("Error: no timeline point matching '{}'", target_id);
        }
        std::process::exit(1);
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
        Some(&format!("restore to {}", target_id)),
        "restore",
        Some(&before_commit),
        Some(&target_ref),
    )?;

    if json {
        println!("{{\"status\": \"restored\", \"target\": \"{}\", \"ref\": \"{}\"}}", target_id, target_ref);
    } else {
        println!("Restored to {} ({})", target_ref, target_id);
    }

    Ok(())
}
