use anyhow::Result;
use std::io::{self, IsTerminal};

use crate::{db, git, snapshot};

pub fn run(title: Option<String>, json: bool) -> Result<()> {
    // Auto-snapshot first
    let conn = db::open(&db::db_path()?)?;
    let _ = snapshot::auto_snapshot(std::path::Path::new("."), &conn);

    // Get the title
    let title = match title {
        Some(t) => t,
        None => {
            // No title provided
            if io::stdin().is_terminal() {
                // TTY available - could open editor, but for MVP just error
                // TODO: open $EDITOR with template
                anyhow::bail!("no change name provided; pass one as argument")
            } else {
                anyhow::bail!("no change name provided; pass one as argument")
            }
        }
    };

    // Create the change
    let (change_id, ref_path, commit_sha) = snapshot::create_change(&conn, &title)?;

    if json {
        println!("{{\"change_id\": \"{}\", \"ref\": \"{}\", \"commit\": \"{}\"}}", change_id, ref_path, commit_sha);
    } else {
        println!("Change created: {} ({})", git::short_sha(&change_id), title);
    }

    Ok(())
}
