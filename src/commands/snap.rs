use anyhow::Result;

use crate::db;
use crate::git;
use crate::id;

pub fn run(message: &str) -> Result<()> {
    let repo = git::open_repo()?;
    let op_id = id::new_op_id();

    // Get current HEAD before snapshot
    let before_commit = git::head_commit_id(&repo)?;

    // Create snapshot commit under hidden ref
    let ref_path = git::create_snapshot(&repo, &op_id, message)?;

    // Record in database
    let db_path = std::path::Path::new(".git/agentvcs/state.sqlite");
    let conn = db::open(db_path)?;
    db::insert_operation(
        &conn,
        &op_id,
        "cli",
        Some(&format!("snap {}", message)),
        Some(&format!("refs/commits/{}", before_commit)),
        Some(&ref_path),
    )?;

    let short_op = &op_id[..op_id.len().min(16)];
    println!("Snapshot created: {} ({})", short_op, message);
    println!("  Ref: {}", ref_path);

    Ok(())
}
