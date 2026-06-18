use anyhow::{Result, bail};

use crate::git;
use crate::oplog;
use crate::output;
use crate::track;

pub fn run(args: Vec<String>, json: bool) -> Result<()> {
    if !git::is_git_repo() {
        bail!("not a git repository");
    }

    if args.is_empty() {
        bail!("no command specified. Usage: avc run -- <command>");
    }

    let root = repo_root()?;
    let branch = git::current_branch()?.unwrap_or_else(|| "HEAD".to_string());

    // Auto-commit before (snapshot)
    track::auto_commit(&root, json)?;
    let before_head = git::head_hash()?.context("no HEAD")?;

    if !json {
        output::info(&format!("running: {}", args.join(" ")));
    }

    // Run the command
    let status = std::process::Command::new(&args[0])
        .args(&args[1..])
        .status()?;

    let exit_code = status.code().unwrap_or(-1);
    let success = status.success();

    // Auto-commit after (snapshot)
    let _after_commit = track::auto_commit(&root, json)?;
    let after_head = git::head_hash()?.context("no HEAD")?;

    // Log to oplog
    let entry = oplog::OpEntry::run(&branch, &after_head, &args.join(" "));
    oplog::append(&root, &entry)?;

    let changed = before_head != after_head;

    if json {
        println!("{{\"exit_code\": {}, \"success\": {}, \"changed\": {}, \"before\": \"{}\", \"after\": \"{}\"}}",
            exit_code, success, changed, before_head, after_head);
    } else {
        if success {
            output::success(&format!("command exited {}", exit_code));
        } else {
            output::warn(&format!("command exited {}", exit_code));
        }
        if changed {
            output::info(&format!("changes detected: {} → {}", 
                output::hash(&before_head), output::hash(&after_head)));
        }
    }

    // Propagate exit code
    if !success {
        std::process::exit(exit_code);
    }

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

use anyhow::Context;
