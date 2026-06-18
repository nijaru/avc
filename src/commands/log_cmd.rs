use anyhow::{Result, bail};

use crate::git;
use crate::oplog;
use crate::output;
use crate::track;

pub fn run(limit: Option<usize>, saves_only: bool, json: bool) -> Result<()> {
    if !git::is_git_repo() {
        bail!("not a git repository");
    }

    let root = repo_root()?;

    // Auto-commit dirty working tree first
    track::auto_commit(&root, json)?;

    let entries = oplog::read_all(&root)?;

    if entries.is_empty() {
        if json {
            println!("[]");
        } else {
            output::info("no operations yet");
        }
        return Ok(());
    }

    // Filter: skip init, optionally skip auto
    let mut display: Vec<&oplog::OpEntry> = entries
        .iter()
        .filter(|e| e.op != "init")
        .filter(|e| !saves_only || e.op == "save" || e.op == "amend")
        .collect();

    if let Some(n) = limit {
        // Take the last N entries
        let skip = display.len().saturating_sub(n);
        display = display.into_iter().skip(skip).collect();
    }

    if json {
        let json_out: Vec<_> = display.iter().map(|e| {
            serde_json::json!({
                "op": e.op,
                "time": e.time,
                "head": e.head,
                "branch": e.branch,
                "title": e.title,
                "files": e.files,
                "command": e.command,
            })
        }).collect();
        println!("{}", serde_json::to_string_pretty(&json_out)?);
        return Ok(());
    }

    // Print timeline
    output::blank();

    for entry in display.iter().rev() {
        let hash = entry.head.as_deref().unwrap_or("?");
        let time = output::time_ago(&entry.time);

        match entry.op.as_str() {
            "auto" => {
                let files = entry.files.as_ref().map(|f| f.join(", ")).unwrap_or_default();
                output::info(&format!("  {} {} {} — {}",
                    output::op_type("auto"),
                    output::hash(hash),
                    files,
                    time));
            }
            "save" | "amend" => {
                let title = entry.title.as_deref().unwrap_or("(no message)");
                let squashed = entry.squashed.as_ref().map(|s| s.len()).unwrap_or(0);
                let amend_tag = if entry.op == "amend" { " (amended)" } else { "" };
                output::info(&format!("◆ {} {} {}{} — {}",
                    output::op_type(&entry.op),
                    output::hash(hash),
                    title,
                    amend_tag,
                    time));
                if squashed > 0 {
                    output::info(&format!("  └─ {} auto-commits", squashed));
                }
            }
            "undo" | "redo" => {
                let from = entry.from.as_deref().unwrap_or("?");
                let to = entry.to.as_deref().unwrap_or("?");
                output::info(&format!("{} {}→{} — {}",
                    output::op_type(&entry.op),
                    output::hash(from),
                    output::hash(to),
                    time));
            }
            "run" => {
                let cmd = entry.command.as_deref().unwrap_or("?");
                output::info(&format!("{} {} {} — {}",
                    output::op_type("run"),
                    output::hash(hash),
                    cmd,
                    time));
            }
            _ => {}
        }
    }

    output::blank();
    Ok(())
}

fn repo_root() -> Result<std::path::PathBuf> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()?;
    if !output.status.success() {
        anyhow::bail!("not in a git repository");
    }
    Ok(std::path::PathBuf::from(
        String::from_utf8_lossy(&output.stdout).trim(),
    ))
}
