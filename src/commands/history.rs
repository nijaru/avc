use anyhow::Result;

use crate::db;

pub fn run(limit: u32) -> Result<()> {
    let db_path = std::path::Path::new(".git/agentvcs/state.sqlite");
    let conn = db::open(db_path)?;
    let ops = db::list_operations(&conn, limit)?;

    if ops.is_empty() {
        println!("No operations recorded yet.");
        return Ok(());
    }

    println!("{:<20} {:<10} {}", "ID", "ACTOR", "COMMAND");
    println!("{}", "-".repeat(60));
    for op in &ops {
        let short_id = if op.id.len() > 16 {
            &op.id[..16]
        } else {
            &op.id
        };
        let cmd = op.command.as_deref().unwrap_or("-");
        println!("{:<20} {:<10} {}", short_id, op.actor, cmd);
    }

    Ok(())
}
