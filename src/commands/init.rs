use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

use crate::{db, id, config};

pub fn run(json: bool) -> Result<()> {
    let repo = gix::open(".").context("not a git repository")?;

    let avc_dir = repo.path().join("agentvcs");
    fs::create_dir_all(&avc_dir)?;

    // Initialize database
    let db_path = avc_dir.join("state.sqlite");
    let conn = db::open(&db_path)?;
    db::initialize_schema(&conn)?;

    // Write default config files to .agentvcs/
    let config_dir = Path::new(".agentvcs");
    fs::create_dir_all(config_dir)?;
    config::write_default_configs(config_dir)?;

    // Install pre-push hook
    install_pre_push_hook(&repo)?;

    // Record init operation
    let op_id = id::new_op_id();
    db::insert_operation(&conn, &op_id, "cli", Some("init"), "init", None, None)?;

    if json {
        println!("{{\"status\": \"initialized\", \"db_path\": \"{}\"}}", db_path.display());
    } else {
        println!("Initialized avc in {}", repo.path().display());
    }

    Ok(())
}

fn install_pre_push_hook(repo: &gix::Repository) -> Result<()> {
    let hooks_dir = repo.path().join("hooks");
    fs::create_dir_all(&hooks_dir)?;

    let hook_path = hooks_dir.join("pre-push");
    let hook_content = r#"#!/bin/sh
# avc pre-push hook: block accidental push of internal refs
while read local_ref local_sha remote_ref remote_sha; do
    case "$remote_ref" in
        refs/agentvcs/*)
            echo "ERROR: Cannot push refs/agentvcs/* refs."
            echo "  avc internal refs should not be pushed to remote."
            echo "  If you really need to push, use: git push --no-verify"
            exit 1
            ;;
    esac
done
"#;

    fs::write(&hook_path, hook_content)?;

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&hook_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&hook_path, perms)?;
    }

    Ok(())
}
