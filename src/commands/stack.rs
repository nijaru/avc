use anyhow::{Result, bail};

use crate::git;
use crate::output;

pub fn run(json: bool) -> Result<()> {
    if !git::is_git_repo() {
        bail!("not a git repository. Run `avc init` first.");
    }

    let entries = find_stack()?;

    if entries.is_empty() {
        if json {
            println!("{{\"saves\": [], \"loose_auto_commits\": 0}}");
        } else {
            output::info("no saves yet — use `avc save` to create one");
        }
        return Ok(());
    }

    let loose = count_loose_auto_commits()?;

    if json {
        let saves: Vec<String> = entries.iter().map(|(h, m)| {
            format!("{{\"hash\": \"{}\", \"message\": \"{}\"}}", h, m.replace('"', "\\\""))
        }).collect();
        println!("{{\"saves\": [{}], \"loose_auto_commits\": {}}}", saves.join(", "), loose);
    } else {
        for (i, (hash, message)) in entries.iter().enumerate() {
            if i == entries.len() - 1 && loose > 0 {
                // Top save with loose auto-commits
                println!("▸ {} {} ({} auto-commits pending)", output::hash(hash), message, loose);
            } else {
                println!("▸ {} {}", output::hash(hash), message);
            }
        }
        if loose > 0 {
            println!("  {} loose auto-commits will be captured on next `avc save`", loose);
        }
    }

    Ok(())
}

fn find_stack() -> Result<Vec<(String, String)>> {
    let mut entries = Vec::new();
    let mut current = "HEAD".to_string();

    loop {
        let msg = git::git(&["log", "-1", "--format=%s", &current])?;
        let msg = msg.trim().to_string();

        // Skip auto-commits and init
        if msg.starts_with("[avc:auto]") || msg.starts_with("[avc:init]") {
            // Continue walking
        } else {
            // This is a save (or manual commit)
            let hash = git::git(&["rev-parse", "--short", &current])?;
            let hash = hash.trim().to_string();

            // Strip Change-Id trailer from display
            let display_msg = msg
                .lines()
                .take_while(|line| !line.starts_with("Change-Id:"))
                .collect::<Vec<_>>()
                .join(" ")
                .trim()
                .to_string();

            entries.push((hash, display_msg));
        }

        // Try parent
        let parent = git::git(&["rev-parse", "--short", &format!("{}^", current)]);
        match parent {
            Ok(p) => current = p.trim().to_string(),
            Err(_) => break,
        }
    }

    entries.reverse();
    Ok(entries)
}

fn count_loose_auto_commits() -> Result<usize> {
    let mut count = 0;
    let mut current = "HEAD".to_string();

    loop {
        let msg = git::git(&["log", "-1", "--format=%s", &current])?;
        let msg = msg.trim();

        if !msg.starts_with("[avc:auto]") {
            break;
        }

        count += 1;

        let parent = git::git(&["rev-parse", "--short", &format!("{}^", current)]);
        match parent {
            Ok(p) => current = p.trim().to_string(),
            Err(_) => break,
        }
    }

    Ok(count)
}
