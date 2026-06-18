use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

/// An entry in the operation log.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum OpEntry {
    #[serde(rename = "init")]
    Init {
        time: String,
        branch: Option<String>,
        head: Option<String>,
    },
    #[serde(rename = "auto")]
    Auto {
        time: String,
        branch: String,
        head: String,
        files: Vec<String>,
    },
    #[serde(rename = "save")]
    Save {
        time: String,
        branch: String,
        head: String,
        title: String,
        squashed: Vec<String>,
    },
    #[serde(rename = "amend")]
    Amend {
        time: String,
        branch: String,
        head: String,
        title: String,
        squashed: Vec<String>,
    },
    #[serde(rename = "undo")]
    Undo {
        time: String,
        from: String,
        to: String,
        target_op: usize,
    },
    #[serde(rename = "redo")]
    Redo {
        time: String,
        from: String,
        to: String,
        target_op: usize,
    },
    #[serde(rename = "run")]
    Run {
        time: String,
        branch: String,
        head: String,
        command: String,
    },
}

impl OpEntry {
    pub fn op_type(&self) -> &str {
        match self {
            OpEntry::Init { .. } => "init",
            OpEntry::Auto { .. } => "auto",
            OpEntry::Save { .. } => "save",
            OpEntry::Amend { .. } => "amend",
            OpEntry::Undo { .. } => "undo",
            OpEntry::Redo { .. } => "redo",
            OpEntry::Run { .. } => "run",
        }
    }

    pub fn time(&self) -> &str {
        match self {
            OpEntry::Init { time, .. } => time,
            OpEntry::Auto { time, .. } => time,
            OpEntry::Save { time, .. } => time,
            OpEntry::Amend { time, .. } => time,
            OpEntry::Undo { time, .. } => time,
            OpEntry::Redo { time, .. } => time,
            OpEntry::Run { time, .. } => time,
        }
    }

    pub fn head(&self) -> Option<&str> {
        match self {
            OpEntry::Init { head, .. } => head.as_deref(),
            OpEntry::Auto { head, .. } => Some(head),
            OpEntry::Save { head, .. } => Some(head),
            OpEntry::Amend { head, .. } => Some(head),
            OpEntry::Undo { to, .. } => Some(to),
            OpEntry::Redo { to, .. } => Some(to),
            OpEntry::Run { head, .. } => Some(head),
        }
    }

    pub fn init(branch: &str, head: Option<&str>) -> Self {
        OpEntry::Init {
            time: chrono::Utc::now().to_rfc3339(),
            branch: Some(branch.to_string()),
            head: head.map(|s| s.to_string()),
        }
    }

    pub fn auto(branch: &str, head: &str, files: Vec<String>) -> Self {
        OpEntry::Auto {
            time: chrono::Utc::now().to_rfc3339(),
            branch: branch.to_string(),
            head: head.to_string(),
            files,
        }
    }

    pub fn save(branch: &str, head: &str, title: &str, squashed: Vec<String>) -> Self {
        OpEntry::Save {
            time: chrono::Utc::now().to_rfc3339(),
            branch: branch.to_string(),
            head: head.to_string(),
            title: title.to_string(),
            squashed,
        }
    }

    pub fn amend(branch: &str, head: &str, title: &str, squashed: Vec<String>) -> Self {
        OpEntry::Amend {
            time: chrono::Utc::now().to_rfc3339(),
            branch: branch.to_string(),
            head: head.to_string(),
            title: title.to_string(),
            squashed,
        }
    }

    pub fn undo(from: &str, to: &str, target_op: usize) -> Self {
        OpEntry::Undo {
            time: chrono::Utc::now().to_rfc3339(),
            from: from.to_string(),
            to: to.to_string(),
            target_op,
        }
    }

    pub fn redo(from: &str, to: &str, target_op: usize) -> Self {
        OpEntry::Redo {
            time: chrono::Utc::now().to_rfc3339(),
            from: from.to_string(),
            to: to.to_string(),
            target_op,
        }
    }

    pub fn run(branch: &str, head: &str, command: &str) -> Self {
        OpEntry::Run {
            time: chrono::Utc::now().to_rfc3339(),
            branch: branch.to_string(),
            head: head.to_string(),
            command: command.to_string(),
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
