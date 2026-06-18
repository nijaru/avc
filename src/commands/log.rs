use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::Connection;

use crate::{db, git};

pub fn run(limit: u32, changes_only: bool, json: bool) -> Result<()> {
    // Auto-snapshot
    let _ = crate::snapshot::auto_snapshot(std::path::Path::new("."), &crate::db::open(&crate::db::db_path()?)?);

    let conn = crate::db::open(&crate::db::db_path()?)?;

    let ops = db::list_operations(&conn, limit)?;

    // Apply changes filter
    let filtered_ops: Vec<_> = if changes_only {
        ops.into_iter().filter(|op| op.kind == "change").collect()
    } else {
        ops
    };

    if json {
        print_json(&filtered_ops, &conn)?;
    } else {
        print_human(&filtered_ops, &conn)?;
    }

    Ok(())
}

fn print_json(ops: &[db::Operation], _conn: &Connection) -> Result<()> {
    let entries: Vec<serde_json::Value> = ops.iter().map(|op| {
        let mut entry = serde_json::json!({
            "id": op.id,
            "kind": op.kind,
            "actor": op.actor,
            "created_at": op.created_at,
        });

        if let Some(ref cmd) = op.command {
            entry["command"] = serde_json::json!(cmd);
        }
        if let Some(ref after) = op.after_ref {
            entry["after_ref"] = serde_json::json!(after);
        }

        entry
    }).collect();

    println!("{}", serde_json::to_string_pretty(&entries)?);
    Ok(())
}

fn print_human(ops: &[db::Operation], _conn: &Connection) -> Result<()> {
    if ops.is_empty() {
        println!("No operations yet. Make some changes and run avc again.");
        return Ok(());
    }

    for op in ops {
        let kind_icon = match op.kind.as_str() {
            "change" => "◆",
            "auto" => "○",
            "undo" => "↺",
            "restore" => "↗",
            "init" => "●",
            _ => "?",
        };

        let time = DateTime::parse_from_rfc3339(&op.created_at)
            .ok()
            .map(|t| human_duration(t.with_timezone(&Utc)))
            .unwrap_or_else(|| "just now".to_string());

        let id_short = git::short_sha(&op.id);

        match op.kind.as_str() {
            "change" => {
                // Get change title from command field
                let title = op.command
                    .as_ref()
                    .and_then(|c| c.strip_prefix("change "))
                    .unwrap_or("untitled");
                println!("{} {} {} — {}", kind_icon, id_short, time, title);
            }
            "auto" => {
                println!("{} {} {}", kind_icon, id_short, time);
            }
            "undo" | "restore" => {
                let cmd = op.command.as_deref().unwrap_or(op.kind.as_str());
                println!("{} {} {} — {}", kind_icon, id_short, time, cmd);
            }
            "init" => {
                println!("{} {} {} — repository initialized", kind_icon, id_short, time);
            }
            _ => {
                println!("{} {} {}", kind_icon, id_short, time);
            }
        }
    }

    Ok(())
}

fn human_duration(dt: DateTime<Utc>) -> String {
    let now = Utc::now();
    let duration = now.signed_duration_since(dt);

    if duration.num_seconds() < 60 {
        "just now".to_string()
    } else if duration.num_minutes() < 60 {
        format!("{} min ago", duration.num_minutes())
    } else if duration.num_hours() < 24 {
        format!("{} hours ago", duration.num_hours())
    } else if duration.num_days() < 30 {
        format!("{} days ago", duration.num_days())
    } else {
        format!("{} months ago", duration.num_days() / 30)
    }
}
