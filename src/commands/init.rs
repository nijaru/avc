use anyhow::{Result, bail};
use std::fs;
use std::path::Path;

use crate::config;
use crate::git;
use crate::oplog;
use crate::output;

pub fn run(json: bool) -> Result<()> {
    if !git::is_git_repo() {
        bail!("not a git repository. Run `git init` first.");
    }

    let root = git::repo_root()?;
    let avc_dir = root.join(".avc");

    if avc_dir.exists() {
        if json {
            println!("{{\"status\": \"already_initialized\"}}");
        } else {
            output::warn("avc already initialized in this repository");
        }
        return Ok(());
    }

    fs::create_dir_all(&avc_dir)?;
    config::write_default(&root)?;
    fs::write(avc_dir.join("oplog"), "")?;
    add_to_gitignore(&root)?;

    // Commit .gitignore to ensure it's always present
    git::add_all()?;
    if git::is_dirty()? {
        git::commit("[avc:init] add .avc/ to .gitignore")?;
    }

    let branch = git::current_branch()?.unwrap_or_else(|| "HEAD".to_string());
    let head = git::head_hash()?;
    let entry = oplog::OpEntry::init(&branch, head.as_deref());
    oplog::append(&root, &entry)?;

    if json {
        println!("{{\"status\": \"initialized\", \"branch\": \"{}\"}}", branch);
    } else {
        output::success("initialized avc");
        output::label_value("branch", &branch);
        if let Some(h) = head {
            output::label_value("head", &h);
        }
    }

    Ok(())
}

fn add_to_gitignore(root: &Path) -> Result<()> {
    let gitignore = root.join(".gitignore");
    let entry = ".avc/";

    let content = if gitignore.exists() {
        fs::read_to_string(&gitignore)?
    } else {
        String::new()
    };

    if !content.lines().any(|line| line.trim() == entry) {
        let mut new_content = content;
        if !new_content.is_empty() && !new_content.ends_with('\n') {
            new_content.push('\n');
        }
        new_content.push_str(entry);
        new_content.push('\n');
        fs::write(&gitignore, new_content)?;
    }

    Ok(())
}
