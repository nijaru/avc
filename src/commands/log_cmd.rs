use anyhow::{Result, bail};

use crate::git;
use crate::oplog;
use crate::output;

/// Log is read-only — does NOT auto-commit.
pub fn run(limit: Option<usize>, saves_only: bool, json: bool) -> Result<()> {
    if !git::is_git_repo() {
        bail!("not a git repository");
    }

    let root = git::repo_root()?;
    let entries = oplog::read_all(&root)?;

    if entries.is_empty() {
        if json {
            println!("[]");
        } else {
            output::info("no operations yet");
        }
        return Ok(());
    }

    let mut display: Vec<&oplog::OpEntry> = entries
        .iter()
        .filter(|e| e.op_type() != "init")
        .filter(|e| !saves_only || e.op_type() == "save" || e.op_type() == "amend")
        .collect();

    if let Some(n) = limit {
        let skip = display.len().saturating_sub(n);
        display = display.into_iter().skip(skip).collect();
    }

    if json {
        let json_out: Vec<_> = display.iter().map(|e| {
            serde_json::json!({
                "op": e.op_type(),
                "time": e.time(),
                "head": e.head(),
            })
        }).collect();
        println!("{}", serde_json::to_string_pretty(&json_out)?);
        return Ok(());
    }

    output::blank();

    for entry in display.iter().rev() {
        let hash = entry.head().unwrap_or("?");
        let time = output::time_ago(entry.time());

        match entry {
            oplog::OpEntry::Auto { files, .. } => {
                output::info(&format!("  {} {} {} — {}",
                    output::op_type("auto"),
                    output::hash(hash),
                    files.join(", "),
                    time));
            }
            oplog::OpEntry::Save { title, squashed, .. } => {
                output::info(&format!("◆ {} {} {} — {}",
                    output::op_type("save"),
                    output::hash(hash),
                    title,
                    time));
                if !squashed.is_empty() {
                    output::info(&format!("  └─ {} auto-commits", squashed.len()));
                }
            }
            oplog::OpEntry::Amend { title, squashed, .. } => {
                output::info(&format!("◆ {} {} {} (amended) — {}",
                    output::op_type("amend"),
                    output::hash(hash),
                    title,
                    time));
                if !squashed.is_empty() {
                    output::info(&format!("  └─ {} auto-commits", squashed.len()));
                }
            }
            oplog::OpEntry::Undo { from, to, .. } => {
                output::info(&format!("{} {}→{} — {}",
                    output::op_type("undo"),
                    output::hash(from),
                    output::hash(to),
                    time));
            }
            oplog::OpEntry::Redo { from, to, .. } => {
                output::info(&format!("{} {}→{} — {}",
                    output::op_type("redo"),
                    output::hash(from),
                    output::hash(to),
                    time));
            }
            oplog::OpEntry::Run { command, .. } => {
                output::info(&format!("{} {} {} — {}",
                    output::op_type("run"),
                    output::hash(hash),
                    command,
                    time));
            }
            oplog::OpEntry::Init { .. } => {}
        }
    }

    output::blank();
    Ok(())
}
