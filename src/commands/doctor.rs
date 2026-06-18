use anyhow::Result;
use std::path::Path;

use crate::{db, git};

pub fn run(json: bool) -> Result<()> {
    // Auto-snapshot (will be skipped by mtime check if no changes)
    let _ = crate::snapshot::auto_snapshot(Path::new("."), &db::open(&db::db_path()?)?);

    let repo = git::open_repo()?;
    let conn = db::open(&db::db_path()?)?;

    let mut issues = Vec::new();

    // Check 1: Database accessible
    match db::open(&db::db_path()?) {
        Ok(_) => {},
        Err(e) => issues.push(format!("Database error: {}", e)),
    }

    // Check 2: Schema valid
    match db::initialize_schema(&conn) {
        Ok(_) => {},
        Err(e) => issues.push(format!("Schema error: {}", e)),
    }

    // Check 3: Pre-push hook installed
    let hooks_dir = repo.path().join("hooks");
    let hook_path = hooks_dir.join("pre-push");
    if !hook_path.exists() {
        issues.push("Pre-push hook not installed".to_string());
    }

    // Check 4: refs/agentvcs/* exists
    let agent_refs = git::list_refs(&repo, "refs/agentvcs/")?;
    if agent_refs.is_empty() {
        issues.push("No agentvcs refs found (no snapshots yet)".to_string());
    }

    // Check 5: Hidden refs don't leak into git log
    let output = std::process::Command::new("git")
        .args(["log", "--oneline", "-5"])
        .output()?;
    let log = String::from_utf8_lossy(&output.stdout);
    if log.contains("[agentvcs:") {
        issues.push("WARNING: agentvcs commits visible in git log (ref leakage!)".to_string());
    }

    if json {
        let status = if issues.is_empty() { "healthy" } else { "issues" };
        println!("{{\"status\": \"{}\", \"issues\": {:?}}}", status, issues);
    } else {
        if issues.is_empty() {
            println!("avc doctor: all checks passed.");
        } else {
            println!("avc doctor: {} issues found:", issues.len());
            for issue in &issues {
                println!("  • {}", issue);
            }
        }
    }

    Ok(())
}
