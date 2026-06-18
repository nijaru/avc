use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

/// An entry in the operation log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpEntry {
    /// Operation type: init, auto, save, amend, undo, redo, run
    pub op: String,
    /// ISO 8601 timestamp
    pub time: String,
    /// Branch name (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// HEAD hash after operation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    /// Files changed (for auto commits)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<String>>,
    /// Commit title (for save/amend)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Commit hash (for save/amend)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// Squashed auto-commit hashes (for save)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub squashed: Option<Vec<String>>,
    /// Command that was run (for run)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// HEAD before operation (for undo/redo)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// HEAD after operation (for undo/redo)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// What operation was undone/redone
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_op: Option<usize>,
}

impl OpEntry {
    pub fn init(branch: &str, head: Option<&str>) -> Self {
        Self {
            op: "init".to_string(),
            time: chrono::Utc::now().to_rfc3339(),
            branch: Some(branch.to_string()),
            head: head.map(|s| s.to_string()),
            files: None,
            title: None,
            commit: None,
            squashed: None,
            command: None,
            from: None,
            to: None,
            target_op: None,
        }
    }

    pub fn auto(branch: &str, head: &str, files: Vec<String>) -> Self {
        Self {
            op: "auto".to_string(),
            time: chrono::Utc::now().to_rfc3339(),
            branch: Some(branch.to_string()),
            head: Some(head.to_string()),
            files: Some(files),
            title: None,
            commit: None,
            squashed: None,
            command: None,
            from: None,
            to: None,
            target_op: None,
        }
    }

    pub fn save(branch: &str, head: &str, title: &str, squashed: Vec<String>) -> Self {
        Self {
            op: "save".to_string(),
            time: chrono::Utc::now().to_rfc3339(),
            branch: Some(branch.to_string()),
            head: Some(head.to_string()),
            files: None,
            title: Some(title.to_string()),
            commit: Some(head.to_string()),
            squashed: Some(squashed),
            command: None,
            from: None,
            to: None,
            target_op: None,
        }
    }

    pub fn amend(branch: &str, head: &str, title: &str, squashed: Vec<String>) -> Self {
        Self {
            op: "amend".to_string(),
            time: chrono::Utc::now().to_rfc3339(),
            branch: Some(branch.to_string()),
            head: Some(head.to_string()),
            files: None,
            title: Some(title.to_string()),
            commit: Some(head.to_string()),
            squashed: Some(squashed),
            command: None,
            from: None,
            to: None,
            target_op: None,
        }
    }

    pub fn undo(from: &str, to: &str, target_op: usize) -> Self {
        Self {
            op: "undo".to_string(),
            time: chrono::Utc::now().to_rfc3339(),
            branch: None,
            head: Some(to.to_string()),
            files: None,
            title: None,
            commit: None,
            squashed: None,
            command: None,
            from: Some(from.to_string()),
            to: Some(to.to_string()),
            target_op: Some(target_op),
        }
    }

    pub fn redo(from: &str, to: &str, target_op: usize) -> Self {
        Self {
            op: "redo".to_string(),
            time: chrono::Utc::now().to_rfc3339(),
            branch: None,
            head: Some(to.to_string()),
            files: None,
            title: None,
            commit: None,
            squashed: None,
            command: None,
            from: Some(from.to_string()),
            to: Some(to.to_string()),
            target_op: Some(target_op),
        }
    }

    pub fn run(branch: &str, head: &str, command: &str) -> Self {
        Self {
            op: "run".to_string(),
            time: chrono::Utc::now().to_rfc3339(),
            branch: Some(branch.to_string()),
            head: Some(head.to_string()),
            files: None,
            title: None,
            commit: None,
            squashed: None,
            command: Some(command.to_string()),
            from: None,
            to: None,
            target_op: None,
        }
    }
}

/// Path to the oplog file.
pub fn oplog_path(repo_root: &Path) -> std::path::PathBuf {
    repo_root.join(".avc").join("oplog")
}

/// Append an entry to the oplog.
pub fn append(repo_root: &Path, entry: &OpEntry) -> Result<()> {
    let path = oplog_path(repo_root);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("failed to open oplog: {}", path.display()))?;

    let line = serde_json::to_string(entry)?;
    writeln!(file, "{}", line)?;
    Ok(())
}

/// Read all oplog entries.
pub fn read_all(repo_root: &Path) -> Result<Vec<OpEntry>> {
    let path = oplog_path(repo_root);
    if !path.exists() {
        return Ok(Vec::new());
    }

    let file = std::fs::File::open(&path)
        .with_context(|| format!("failed to open oplog: {}", path.display()))?;

    let reader = BufReader::new(file);
    let mut entries = Vec::new();

    for (i, line) in reader.lines().enumerate() {
        let line = line.with_context(|| format!("failed to read oplog line {}", i + 1))?;
        if line.trim().is_empty() {
            continue;
        }
        let entry: OpEntry = serde_json::from_str(&line)
            .with_context(|| format!("failed to parse oplog line {}", i + 1))?;
        entries.push(entry);
    }

    Ok(entries)
}

/// Get the last entry of a given op type.
pub fn last_of_type(repo_root: &Path, op_type: &str) -> Result<Option<OpEntry>> {
    let entries = read_all(repo_root)?;
    Ok(entries.into_iter().rev().find(|e| e.op == op_type))
}
