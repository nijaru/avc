use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha1::Digest;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

/// Generate a Change-Id in Gerrit-compatible format.
/// Format: "I" followed by 40 hex characters (SHA-1 of UUID).
pub fn generate_change_id() -> String {
    let uuid = uuid::Uuid::new_v4();
    let hash = sha1::Sha1::digest(uuid.as_bytes());
    let hash_hex: String = hash.iter().map(|b| format!("{:02x}", b)).collect();
    format!("I{}", hash_hex)
}

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
        change_id: Option<String>,
    },
    #[serde(rename = "amend")]
    Amend {
        time: String,
        branch: String,
        head: String,
        title: String,
        squashed: Vec<String>,
        change_id: Option<String>,
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

    pub fn save(branch: &str, head: &str, title: &str, squashed: Vec<String>, change_id: Option<String>) -> Self {
        OpEntry::Save {
            time: chrono::Utc::now().to_rfc3339(),
            branch: branch.to_string(),
            head: head.to_string(),
            title: title.to_string(),
            squashed,
            change_id,
        }
    }

    pub fn amend(branch: &str, head: &str, title: &str, squashed: Vec<String>, change_id: Option<String>) -> Self {
        OpEntry::Amend {
            time: chrono::Utc::now().to_rfc3339(),
            branch: branch.to_string(),
            head: head.to_string(),
            title: title.to_string(),
            squashed,
            change_id,
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (TempDir, std::path::PathBuf) {
        let dir = TempDir::new().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::create_dir_all(root.join(".avc")).unwrap();
        (dir, root)
    }

    #[test]
    fn test_opentry_auto_roundtrip() {
        let entry = OpEntry::auto("main", "abc1234", vec!["file1.txt".to_string(), "file2.txt".to_string()]);
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: OpEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.op_type(), "auto");
        assert_eq!(parsed.head(), Some("abc1234"));
        assert_eq!(parsed.time(), entry.time());
    }

    #[test]
    fn test_opentry_save_roundtrip() {
        let entry = OpEntry::save("main", "def5678", "my save", vec!["abc1234".to_string()], Some("Iabc123".to_string()));
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: OpEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.op_type(), "save");
        assert_eq!(parsed.head(), Some("def5678"));
    }

    #[test]
    fn test_opentry_amend_roundtrip() {
        let entry = OpEntry::amend("main", "ghi9012", "amended", vec!["abc1234".to_string()], Some("Iabc123".to_string()));
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: OpEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.op_type(), "amend");
        assert_eq!(parsed.head(), Some("ghi9012"));
    }

    #[test]
    fn test_opentry_undo_roundtrip() {
        let entry = OpEntry::undo("abc1234", "def5678", 5);
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: OpEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.op_type(), "undo");
        assert_eq!(parsed.head(), Some("def5678"));
    }

    #[test]
    fn test_opentry_redo_roundtrip() {
        let entry = OpEntry::redo("abc1234", "def5678", 3);
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: OpEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.op_type(), "redo");
        assert_eq!(parsed.head(), Some("def5678"));
    }

    #[test]
    fn test_opentry_run_roundtrip() {
        let entry = OpEntry::run("main", "abc1234", "cargo test");
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: OpEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.op_type(), "run");
        assert_eq!(parsed.head(), Some("abc1234"));
    }

    #[test]
    fn test_opentry_init_roundtrip() {
        let entry = OpEntry::init("main", Some("abc1234"));
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: OpEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.op_type(), "init");
        assert_eq!(parsed.head(), Some("abc1234"));
    }

    #[test]
    fn test_opentry_init_no_head() {
        let entry = OpEntry::init("main", None);
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: OpEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.op_type(), "init");
        assert_eq!(parsed.head(), None);
    }

    #[test]
    fn test_append_and_read_all() {
        let (_dir, root) = setup();
        
        let entry1 = OpEntry::init("main", Some("abc1234"));
        let entry2 = OpEntry::auto("main", "def5678", vec!["file.txt".to_string()]);
        let entry3 = OpEntry::save("main", "ghi9012", "my save", vec!["def5678".to_string()], Some("Iabc123".to_string()));
        
        append(&root, &entry1).unwrap();
        append(&root, &entry2).unwrap();
        append(&root, &entry3).unwrap();
        
        let entries = read_all(&root).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].op_type(), "init");
        assert_eq!(entries[1].op_type(), "auto");
        assert_eq!(entries[2].op_type(), "save");
    }

    #[test]
    fn test_read_all_empty() {
        let (_dir, root) = setup();
        let entries = read_all(&root).unwrap();
        assert_eq!(entries.len(), 0);
    }

    #[test]
    fn test_read_all_nonexistent() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().to_path_buf();
        let entries = read_all(&root).unwrap();
        assert_eq!(entries.len(), 0);
    }

    #[test]
    fn test_read_all_skips_empty_lines() {
        let (_dir, root) = setup();
        let path = oplog_path(&root);
        
        let entry = OpEntry::init("main", Some("abc1234"));
        let json = serde_json::to_string(&entry).unwrap();
        
        std::fs::write(&path, format!("{}\n\n{}\n", json, json)).unwrap();
        
        let entries = read_all(&root).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn test_read_all_fails_on_invalid_json() {
        let (_dir, root) = setup();
        let path = oplog_path(&root);
        
        std::fs::write(&path, "invalid json\n").unwrap();
        
        let result = read_all(&root);
        assert!(result.is_err());
    }

    #[test]
    fn test_append_creates_file() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::create_dir_all(root.join(".avc")).unwrap();
        
        let entry = OpEntry::init("main", Some("abc1234"));
        append(&root, &entry).unwrap();
        
        let path = oplog_path(&root);
        assert!(path.exists());
    }

    #[test]
    fn test_append_preserves_existing() {
        let (_dir, root) = setup();
        
        let entry1 = OpEntry::init("main", Some("abc1234"));
        let entry2 = OpEntry::auto("main", "def5678", vec!["file.txt".to_string()]);
        
        append(&root, &entry1).unwrap();
        append(&root, &entry2).unwrap();
        
        let entries = read_all(&root).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].op_type(), "init");
        assert_eq!(entries[1].op_type(), "auto");
    }
}
