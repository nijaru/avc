use anyhow::Result;

use crate::{db, git};

pub fn run(json: bool) -> Result<()> {
    // Auto-snapshot
    let _ = crate::snapshot::auto_snapshot(std::path::Path::new("."), &crate::db::open(&crate::db::db_path()?)?);

    let conn = crate::db::open(&crate::db::db_path()?)?;
    let repo = git::open_repo()?;

    let branch = git::current_branch(&repo)?;
    let head = git::head_commit_id(&repo)?
        .map(|h| git::short_sha(&h).to_string())
        .unwrap_or_else(|| "no commits yet".to_string());

    let latest_change = db::get_latest_change(&conn)?;

    if json {
        print_json(&branch, &head, latest_change.as_ref())?;
    } else {
        print_human(&branch, &head, latest_change.as_ref(), &repo)?;
    }

    Ok(())
}

fn print_json(
    branch: &str,
    head: &str,
    latest_change: Option<&db::Change>,
) -> Result<()> {
    let mut output = serde_json::json!({
        "branch": branch,
        "head": head,
    });

    if let Some(change) = latest_change {
        output["latest_change"] = serde_json::json!({
            "id": change.id,
            "title": change.title,
            "status": change.status,
        });
    }

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn print_human(
    branch: &str,
    head: &str,
    latest_change: Option<&db::Change>,
    repo: &git::Repository,
) -> Result<()> {
    println!("On branch {}, HEAD at {}", branch, head);

    if let Some(change) = latest_change {
        println!("Last change: {} — {}", git::short_sha(&change.id), change.title);
    } else {
        println!("No named changes yet.");
    }

    // Show diff stats since last change
    if let Some(change) = latest_change {
        if let Some(ref commit) = change.current_commit {
            let tree_sha = git::commit_tree_sha(repo, commit)?;
            let output = std::process::Command::new("git")
                .args(["diff", "--stat", &format!("{}^{{tree}}", tree_sha)])
                .output()?;

            if !output.stdout.is_empty() {
                println!("\nChanges since last change:");
                println!("{}", String::from_utf8_lossy(&output.stdout));
            }
        }
    } else {
        // Show diff against HEAD
        let output = std::process::Command::new("git")
            .args(["diff", "--stat", "HEAD"])
            .output()?;

        if !output.stdout.is_empty() {
            println!("\nChanges since HEAD:");
            println!("{}", String::from_utf8_lossy(&output.stdout));
        }
    }

    Ok(())
}
