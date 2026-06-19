use anyhow::{Result, bail};
use std::time::Duration;

use crate::git;
use crate::output;
use crate::track;

pub fn run(interval: u64, json: bool) -> Result<()> {
    if !git::is_git_repo() {
        bail!("not a git repository");
    }

    let root = git::repo_root()?;

    if json {
        println!("{{\"status\": \"watching\", \"interval\": {}}}", interval);
    } else {
        output::info(&format!("watching for changes (every {}s, Ctrl+C to stop)", interval));
    }

    let poll_interval = Duration::from_secs(interval);
    let mut last_hash = git::head_hash()?.unwrap_or_default();

    loop {
        std::thread::sleep(poll_interval);

        // Check if working tree has changes by looking at status
        let status = git::git(&["status", "--porcelain"]);
        let has_changes = match &status {
            Ok(s) => !s.trim().is_empty(),
            Err(_) => false,
        };

        if has_changes {
            match track::auto_commit(&root, json) {
                Ok(hash) => {
                    if let Some(h) = hash {
                        if h != last_hash {
                            if json {
                                println!("{{\"status\": \"auto_committed\", \"hash\": \"{}\"}}", h);
                            } else {
                                output::info(&format!("auto-saved {}", output::hash(&h)));
                            }
                            last_hash = h;
                        }
                    }
                }
                Err(e) => {
                    // Silently ignore "nothing to commit"
                    if !format!("{}", e).contains("nothing to commit") {
                        if json {
                            eprintln!("{{\"error\": \"auto-commit failed: {}\"}}", e);
                        } else {
                            output::error(&format!("auto-commit failed: {}", e));
                        }
                    }
                }
            }
        }
    }
}
