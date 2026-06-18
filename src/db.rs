use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{Connection, params};
use std::path::PathBuf;

/// Get the path to the avc database.
pub fn db_path() -> Result<PathBuf> {
    let repo = gix::open(".").context("not a git repository")?;
    Ok(repo.path().join("agentvcs").join("state.sqlite"))
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS operations (
    id TEXT PRIMARY KEY,
    actor TEXT NOT NULL,
    command TEXT,
    kind TEXT NOT NULL,
    before_commit TEXT,
    after_ref TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS changes (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'draft',
    current_commit TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS change_commits (
    change_id TEXT NOT NULL,
    git_commit TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT 'current',
    created_at TEXT NOT NULL,
    PRIMARY KEY (change_id, git_commit)
);

CREATE TABLE IF NOT EXISTS lanes (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    type TEXT NOT NULL,
    git_branch TEXT,
    git_worktree_path TEXT,
    base_ref TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS stacks (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    target_ref TEXT NOT NULL,
    export_strategy TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS stack_items (
    stack_id TEXT NOT NULL,
    change_id TEXT NOT NULL,
    position INTEGER NOT NULL,
    PRIMARY KEY (stack_id, change_id)
);

CREATE TABLE IF NOT EXISTS agent_runs (
    id TEXT PRIMARY KEY,
    lane_id TEXT,
    task TEXT,
    tool TEXT,
    model TEXT,
    prompt_digest TEXT,
    redacted_summary TEXT,
    status TEXT NOT NULL DEFAULT 'running',
    created_at TEXT NOT NULL,
    completed_at TEXT
);

CREATE TABLE IF NOT EXISTS agent_run_files (
    run_id TEXT NOT NULL,
    path TEXT NOT NULL,
    access_type TEXT NOT NULL,
    PRIMARY KEY (run_id, path, access_type)
);

CREATE TABLE IF NOT EXISTS validations (
    id TEXT PRIMARY KEY,
    change_id TEXT,
    stack_id TEXT,
    command TEXT NOT NULL,
    status TEXT NOT NULL,
    output_digest TEXT,
    environment_digest TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS exports (
    id TEXT PRIMARY KEY,
    change_id TEXT,
    stack_id TEXT,
    provider TEXT NOT NULL,
    pr_url TEXT,
    branch TEXT,
    last_exported_commit TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
";

pub fn open(db_path: &std::path::Path) -> Result<Connection> {
    let conn = Connection::open(db_path)
        .with_context(|| format!("failed to open database at {}", db_path.display()))?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    Ok(conn)
}

pub fn initialize_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA)
        .context("failed to create database schema")?;
    Ok(())
}

// --- Operations ---

#[allow(dead_code)]
pub struct Operation {
    pub id: String,
    pub actor: String,
    pub command: Option<String>,
    pub kind: String,
    pub before_commit: Option<String>,
    pub after_ref: Option<String>,
    pub created_at: String,
}

pub fn insert_operation(
    conn: &Connection,
    id: &str,
    actor: &str,
    command: Option<&str>,
    kind: &str,
    before_commit: Option<&str>,
    after_ref: Option<&str>,
) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO operations (id, actor, command, kind, before_commit, after_ref, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![id, actor, command, kind, before_commit, after_ref, now],
    )?;
    Ok(())
}

pub fn list_operations(conn: &Connection, limit: u32) -> Result<Vec<Operation>> {
    let mut stmt = conn.prepare(
        "SELECT id, actor, command, kind, before_commit, after_ref, created_at FROM operations ORDER BY created_at DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], |row| {
        Ok(Operation {
            id: row.get(0)?,
            actor: row.get(1)?,
            command: row.get(2)?,
            kind: row.get(3)?,
            before_commit: row.get(4)?,
            after_ref: row.get(5)?,
            created_at: row.get(6)?,
        })
    })?;
    let mut ops = Vec::new();
    for row in rows {
        ops.push(row?);
    }
    Ok(ops)
}

/// Get the latest operation with an after_ref (for finding previous snapshot).
pub fn get_latest_ref_operation(conn: &Connection, exclude_id: &str) -> Result<Option<Operation>> {
    let result = conn.query_row(
        "SELECT id, actor, command, kind, before_commit, after_ref, created_at FROM operations WHERE after_ref IS NOT NULL AND id != ?1 ORDER BY created_at DESC LIMIT 1",
        params![exclude_id],
        |row| {
            Ok(Operation {
                id: row.get(0)?,
                actor: row.get(1)?,
                command: row.get(2)?,
                kind: row.get(3)?,
                before_commit: row.get(4)?,
                after_ref: row.get(5)?,
                created_at: row.get(6)?,
            })
        },
    );
    match result {
        Ok(op) => Ok(Some(op)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Count auto-snapshot operations.
pub fn count_auto_snapshots(conn: &Connection) -> Result<u32> {
    let count: u32 = conn.query_row(
        "SELECT COUNT(*) FROM operations WHERE kind = 'auto'",
        [],
        |row| row.get(0),
    )?;
    Ok(count)
}

/// Get the oldest auto-snapshot ref for pruning.
pub fn get_oldest_auto_ref(conn: &Connection) -> Result<Option<String>> {
    let result = conn.query_row(
        "SELECT after_ref FROM operations WHERE kind = 'auto' AND after_ref IS NOT NULL ORDER BY created_at ASC LIMIT 1",
        [],
        |row| row.get::<_, String>(0),
    );
    match result {
        Ok(ref_path) => Ok(Some(ref_path)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Get an operation by prefix match on id.
pub fn get_operation_by_prefix(conn: &Connection, prefix: &str) -> Result<Option<Operation>> {
    let result = conn.query_row(
        "SELECT id, actor, command, kind, before_commit, after_ref, created_at FROM operations WHERE id LIKE ?1 || '%' LIMIT 1",
        params![prefix],
        |row| {
            Ok(Operation {
                id: row.get(0)?,
                actor: row.get(1)?,
                command: row.get(2)?,
                kind: row.get(3)?,
                before_commit: row.get(4)?,
                after_ref: row.get(5)?,
                created_at: row.get(6)?,
            })
        },
    );
    match result {
        Ok(op) => Ok(Some(op)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

// --- Changes ---

pub struct Change {
    pub id: String,
    pub title: String,
    pub status: String,
    pub current_commit: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub fn insert_change(conn: &Connection, id: &str, title: &str, current_commit: &str) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO changes (id, title, status, current_commit, created_at, updated_at) VALUES (?1, ?2, 'draft', ?3, ?4, ?5)",
        params![id, title, current_commit, now, now],
    )?;
    conn.execute(
        "INSERT INTO change_commits (change_id, git_commit, role, created_at) VALUES (?1, ?2, 'current', ?3)",
        params![id, current_commit, now],
    )?;
    Ok(())
}

pub fn get_latest_change(conn: &Connection) -> Result<Option<Change>> {
    let result = conn.query_row(
        "SELECT id, title, status, current_commit, created_at, updated_at FROM changes ORDER BY created_at DESC LIMIT 1",
        [],
        |row| {
            Ok(Change {
                id: row.get(0)?,
                title: row.get(1)?,
                status: row.get(2)?,
                current_commit: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        },
    );
    match result {
        Ok(change) => Ok(Some(change)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Get a change by prefix match on id.
pub fn get_change_by_prefix(conn: &Connection, prefix: &str) -> Result<Option<Change>> {
    let result = conn.query_row(
        "SELECT id, title, status, current_commit, created_at, updated_at FROM changes WHERE id LIKE ?1 || '%' LIMIT 1",
        params![prefix],
        |row| {
            Ok(Change {
                id: row.get(0)?,
                title: row.get(1)?,
                status: row.get(2)?,
                current_commit: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        },
    );
    match result {
        Ok(change) => Ok(Some(change)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn list_changes(conn: &Connection) -> Result<Vec<Change>> {
    let mut stmt = conn.prepare(
        "SELECT id, title, status, current_commit, created_at, updated_at FROM changes ORDER BY created_at DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(Change {
            id: row.get(0)?,
            title: row.get(1)?,
            status: row.get(2)?,
            current_commit: row.get(3)?,
            created_at: row.get(4)?,
            updated_at: row.get(5)?,
        })
    })?;
    let mut changes = Vec::new();
    for row in rows {
        changes.push(row?);
    }
    Ok(changes)
}

pub fn get_change(conn: &Connection, id: &str) -> Result<Option<Change>> {
    let result = conn.query_row(
        "SELECT id, title, status, current_commit, created_at, updated_at FROM changes WHERE id = ?1",
        params![id],
        |row| {
            Ok(Change {
                id: row.get(0)?,
                title: row.get(1)?,
                status: row.get(2)?,
                current_commit: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        },
    );
    match result {
        Ok(change) => Ok(Some(change)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}
