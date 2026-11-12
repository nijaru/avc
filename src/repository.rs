//! Experimental local storage and authored history, using SQLite WAL/FULL.
//!
//! SQLite WAL/FULL owns canonical payloads, the selection and immutable receipts
//! in the same transaction.
//! Compression, extents, large-object streaming, deletion and GC are deferred.
//! All closure operations buffer bounded data (and may hold several copies).
//! No leases are needed while retained object rows are never removed or changed.
//!
//! Requires a trusted administration directory and a local filesystem honoring
//! SQLite locking, file sync and directory sync. AVC-owned sync failures are
//! fatal; SQLite may internally fall back from macOS F_FULLFSYNC to fsync.
//! OS/device power-loss behavior and hostile directory replacement
//! have not been qualified. macOS uses F_FULLFSYNC as well as SQLite fullfsync.
//!
//! Object-only export v1 is `AVCEXP01`, a 33-byte typed root, a little-endian
//! u64 record count, then sorted records `(typed ID[33], length u64 LE, payload)`.
//! `AVCEND01` and a 32-byte BLAKE3 digest of every preceding byte complete the
//! stream; EOF is mandatory. This is not a backup of mutable repository state.

use crate::{
    Entry, Id, Node, Object,
    engine::{Engine, Limits},
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

pub type Result<T> = std::result::Result<T, Error>;

/// Physical implementation limits, not canonical format restrictions.
pub const MAX_OBJECT_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_CLOSURE_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_OBJECTS: usize = 100_000;
const MAX_REQUEST_BYTES: usize = 1024;
const APPLICATION_ID: i64 = 0x41564331;
const SCHEMA_VERSION: i64 = 7;

mod git_bridge;
pub use git_bridge::GitOrigin;
mod history;
mod rebase;
mod transfer;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Sqlite(rusqlite::Error),
    Source(crate::Error),
    AlreadyExists(PathBuf),
    Invalid(String),
    Limit(&'static str),
    Stale { expected: u64, actual: u64 },
    RequestKeyReuse(String),
    NotEmpty,
    RecoveryRequired,
    DurabilityPending,
    MissingObject(Id),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => e.fmt(f),
            Self::Sqlite(e) => e.fmt(f),
            Self::Source(e) => e.fmt(f),
            Self::AlreadyExists(p) => write!(f, "repository path already exists: {}", p.display()),
            Self::Invalid(s) => write!(f, "invalid repository data: {s}"),
            Self::Limit(s) => write!(f, "repository resource limit: {s}"),
            Self::Stale { expected, actual } => {
                write!(f, "stale selection: expected {expected}, actual {actual}")
            }
            Self::RequestKeyReuse(key) => {
                write!(f, "request key reused with a different identity: {key}")
            }
            Self::NotEmpty => f.write_str("import requires an untouched initialized repository"),
            Self::RecoveryRequired => {
                f.write_str("unfinished SQLite transaction: drop this repository handle and reopen")
            }
            Self::DurabilityPending => f.write_str(
                "WAL durability checkpoint incomplete: retry after active readers/writers release",
            ),
            Self::MissingObject(id) => write!(f, "missing retained object {id}"),
        }
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Sqlite(e) => Some(e),
            Self::Source(e) => Some(e),
            _ => None,
        }
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}
impl From<crate::Error> for Error {
    fn from(e: crate::Error) -> Self {
        Self::Source(e)
    }
}
fn invalid(s: &str) -> Error {
    Error::Invalid(s.to_owned())
}
fn limits() -> Limits {
    Limits {
        objects: MAX_OBJECTS,
        bindings: 1_000_000,
        bytes: MAX_CLOSURE_BYTES,
    }
}

/// The selected source and last authored revision share one CAS generation.
/// Source edits preserve the revision; initialization authors no revision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Selection {
    pub root: Id,
    pub generation: u64,
    pub revision: Option<Id>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Operation {
    Source,
    Commit,
    Amend,
    Rebase { onto: Id },
    ImportHistory,
    ImportGit { commit: crate::git::GitOid },
    Undo { target: String },
}
impl Operation {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Commit => "commit",
            Self::Amend => "amend",
            Self::Rebase { .. } => "rebase",
            Self::ImportHistory => "import-history",
            Self::ImportGit { .. } => "import-git",
            Self::Undo { .. } => "undo",
        }
    }
    fn git_commit(&self) -> Option<crate::git::GitOid> {
        match self {
            Self::ImportGit { commit } => Some(*commit),
            _ => None,
        }
    }
    fn rebase_onto(&self) -> Option<Id> {
        match self {
            Self::Rebase { onto } => Some(*onto),
            _ => None,
        }
    }
    fn undo_target(&self) -> Option<&str> {
        match self {
            Self::Undo { target } => Some(target),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
enum Mutation<'a> {
    Source {
        root: Id,
        empty_only: bool,
    },
    Author {
        author: &'a str,
        message: &'a str,
        amend: bool,
    },
    ImportHistory {
        root: Id,
        revision: Option<Id>,
    },
    ImportGit {
        root: Id,
        origin: &'a GitOrigin,
    },
    Undo {
        target: &'a str,
    },
    Rebase {
        onto: Id,
        root: Id,
        revision: Id,
    },
}
impl Mutation<'_> {
    fn operation(self) -> Operation {
        match self {
            Self::Source { .. } => Operation::Source,
            Self::Author { amend, .. } => {
                if amend {
                    Operation::Amend
                } else {
                    Operation::Commit
                }
            }
            Self::ImportHistory { .. } => Operation::ImportHistory,
            Self::ImportGit { origin, .. } => Operation::ImportGit {
                commit: origin.commit,
            },
            Self::Rebase { onto, .. } => Operation::Rebase { onto },
            Self::Undo { target } => Operation::Undo {
                target: target.to_owned(),
            },
        }
    }
}

/// Immutable private retention of both selections. Not exported by `export`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Receipt {
    pub request_key: String,
    pub operation: Operation,
    pub before: Selection,
    pub after: Selection,
}

pub struct Repository {
    connection: Connection,
}

impl Repository {
    /// Exclusively creates `.avc`; refuses any existing entry, including dangling
    /// symlinks. On a later initialization error the partial directory is left
    /// intact for inspection, never silently removed or reinitialized.
    pub fn init(project_dir: &Path) -> Result<Self> {
        let directory = project_dir.join(".avc");
        match fs::create_dir(&directory) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(Error::AlreadyExists(directory));
            }
            Err(e) => return Err(e.into()),
        }
        let mut connection = Connection::open_with_flags(
            directory.join("store.sqlite"),
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )?;
        configure(&connection, true)?;
        let mut engine = Engine::new();
        let root = engine.empty()?;
        let payload = engine.object(root)?.encode()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(SCHEMA)?;
        tx.pragma_update(None, "application_id", APPLICATION_ID)?;
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        tx.execute("INSERT INTO repository VALUES (1, randomblob(16))", [])?;
        tx.execute(
            "INSERT INTO objects VALUES (?1, ?2, ?3)",
            params![root.to_bytes()?.as_slice(), payload.len() as i64, payload],
        )?;
        tx.execute(
            "INSERT INTO selection VALUES (1, ?1, 0, NULL)",
            [root.to_bytes()?.as_slice()],
        )?;
        tx.commit()?;
        sync_repository_installation(&directory, project_dir)?;
        durability_fence(&connection)?;
        Ok(Self { connection })
    }

    /// Opens only a recognized existing database; never creates a missing DB.
    pub fn open(project_dir: &Path) -> Result<Self> {
        let directory = project_dir.join(".avc");
        require_directory(&directory)?;
        require_file(&directory.join("store.sqlite"))?;
        let connection = Connection::open_with_flags(
            directory.join("store.sqlite"),
            OpenFlags::SQLITE_OPEN_READ_WRITE,
        )?;
        let app: i64 = connection.pragma_query_value(None, "application_id", |r| r.get(0))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if app != APPLICATION_ID {
            return Err(invalid("unknown database format"));
        }
        if version != SCHEMA_VERSION {
            return Err(Error::Invalid(format!(
                "unsupported physical schema {version}; expected {SCHEMA_VERSION}; automatic migration is not supported"
            )));
        }
        configure(&connection, false)?;
        let valid: bool = connection.query_row(
            "SELECT length(instance) = 16 FROM repository WHERE singleton = 1",
            [],
            |r| r.get(0),
        )?;
        if !valid {
            return Err(invalid("repository instance"));
        }
        let repo = Self { connection };
        repo.status()?;
        // A creator can fail or die after schema commit but before directory
        // installation is synced. Every opener completes that idempotent phase
        // before returning a handle that could acknowledge a new root.
        sync_repository_installation(&directory, project_dir)?;
        durability_fence(&repo.connection)?;
        Ok(repo)
    }

    fn ensure_idle(&self) -> Result<()> {
        if !self.connection.is_autocommit() {
            return Err(Error::RecoveryRequired);
        }
        Ok(())
    }

    /// Metadata only: does not assert physical availability or source custody.
    pub fn status(&self) -> Result<Selection> {
        self.ensure_idle()?;
        selection(&self.connection)
    }

    /// A visible recovered WAL record is not by itself a durable acknowledgement.
    /// Complete the checked SQLite fence before reporting a committed receipt.
    pub fn receipt(&self, request_key: &str) -> Result<Option<Receipt>> {
        self.ensure_idle()?;
        let prior = receipt(&self.connection, request_key)?;
        if prior.is_some() {
            durability_fence(&self.connection)?;
        }
        Ok(prior)
    }

    /// Resolve the original source base for a conditional request/retry. Receipt
    /// and selection are one SQLite read snapshot: an identical concurrent
    /// commit cannot turn a valid preparation into a spurious Stale result.
    pub fn preparation_base(
        &self,
        expected_generation: u64,
        request_key: &str,
    ) -> Result<Selection> {
        self.ensure_idle()?;
        check_request_key(request_key)?;
        let tx = self.connection.unchecked_transaction()?;
        let previous = match receipt(&tx, request_key)? {
            Some(prior) => {
                if prior.operation != Operation::Source
                    || prior.before.generation != expected_generation
                {
                    return Err(Error::RequestKeyReuse(request_key.to_owned()));
                }
                prior.before
            }
            None => selection(&tx)?,
        };
        if previous.generation != expected_generation {
            return Err(Error::Stale {
                expected: expected_generation,
                actual: previous.generation,
            });
        }
        tx.commit()?;
        Ok(previous)
    }

    /// Reads one selection snapshot and fully verifies its exact owning closure.
    /// Concurrent selection advances cannot invalidate the immutable locations.
    pub fn load(&self) -> Result<(Selection, Engine)> {
        let selected = self.status()?;
        Ok((selected, self.load_root(selected.root)?))
    }

    pub fn load_root(&self, root: Id) -> Result<Engine> {
        self.ensure_idle()?;
        let records = collect_closure([root], |id| self.read_object(id))?;
        let mut engine = Engine::new();
        engine.import(records, root, limits())?;
        Ok(engine)
    }

    /// Durably admits only the exact owning closure, then atomically advances
    /// the selection and retains before/after roots in a receipt. Request identity
    /// is `(request_key, root, expected_generation)`. A committed retry returns
    /// its original receipt before stale checks or consulting the supplied engine.
    /// Object payloads, selection and receipt commit in one SQLite transaction.
    pub fn commit_source(
        &mut self,
        engine: &Engine,
        root: Id,
        expected_generation: u64,
        request_key: &str,
    ) -> Result<Receipt> {
        self.ensure_idle()?;
        check_request_key(request_key)?;
        let mutation = Mutation::Source {
            root,
            empty_only: false,
        };
        if let Some(prior) = self.receipt(request_key)? {
            return self.reconcile(prior, expected_generation, mutation);
        }
        require_source_root(root)?;
        i64::try_from(expected_generation).map_err(|_| Error::Limit("generation"))?;
        // Refuse known stale work before preparing data. Recheck under the write
        // transaction, including a receipt created by a concurrent same request.
        let current = self.status()?;
        if current.generation != expected_generation {
            // Reconcile an identical request that won the race to selection.
            if let Some(prior) = self.receipt(request_key)? {
                return self.reconcile(prior, expected_generation, mutation);
            }
            return Err(Error::Stale {
                expected: expected_generation,
                actual: current.generation,
            });
        }
        let records = collect_closure([root], |id| encode_bounded(engine.object(id)?))?;
        engine.verify(root, limits())?;
        let prepared = self.prepare_objects(records)?;
        self.publish(expected_generation, request_key, prepared, mutation)
    }

    fn prepare_objects(&self, records: BTreeMap<Id, Vec<u8>>) -> Result<Vec<(Id, Vec<u8>)>> {
        let mut prepared = Vec::new();
        for (id, payload) in records {
            if let Some(stored) = read_object_optional(&self.connection, id)? {
                if stored != payload {
                    return Err(invalid("object collision"));
                }
            } else {
                prepared.push((id, payload));
            }
        }
        Ok(prepared)
    }

    fn publish(
        &mut self,
        expected_generation: u64,
        request_key: &str,
        prepared: Vec<(Id, Vec<u8>)>,
        mutation: Mutation<'_>,
    ) -> Result<Receipt> {
        let expected =
            i64::try_from(expected_generation).map_err(|_| Error::Limit("generation"))?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(prior) = receipt(&tx, request_key)? {
            tx.rollback()?;
            durability_fence(&self.connection)?;
            return self.reconcile(prior, expected_generation, mutation);
        }
        let before = selection(&tx)?;
        if matches!(
            mutation,
            Mutation::Source {
                empty_only: true,
                ..
            } | Mutation::ImportHistory { .. }
                | Mutation::ImportGit { .. }
        ) && before.generation != 0
        {
            return Err(Error::NotEmpty);
        }
        if before.generation != expected_generation {
            return Err(Error::Stale {
                expected: expected_generation,
                actual: before.generation,
            });
        }
        let next = expected
            .checked_add(1)
            .ok_or(Error::Limit("generation exhausted"))?;
        // Previously verified rows cannot change or disappear. Keep full reads
        // outside the write transaction except for new concurrent dedup winners.
        insert_objects(&tx, prepared)?;
        let after = match mutation {
            Mutation::ImportHistory { root, revision } => Selection {
                root,
                revision,
                generation: next as u64,
            },
            Mutation::ImportGit { root, origin } => {
                git_bridge::insert_origin(&tx, origin)?;
                Selection {
                    root,
                    revision: Some(origin.revision),
                    generation: next as u64,
                }
            }
            Mutation::Rebase { root, revision, .. } => Selection {
                root,
                revision: Some(revision),
                generation: next as u64,
            },
            Mutation::Source { root, .. } => Selection {
                root,
                generation: next as u64,
                revision: before.revision,
            },
            Mutation::Undo { target } => {
                let restored = history::undo_selection(&tx, before, target)?;
                Selection {
                    generation: next as u64,
                    ..restored
                }
            }
            Mutation::Author {
                author,
                message,
                amend,
            } => {
                let (id, payload) = history::prepare(&tx, before, author, message, amend)?;
                insert_objects(&tx, [(id, payload)])?;
                Selection {
                    root: before.root,
                    generation: next as u64,
                    revision: Some(id),
                }
            }
        };
        let before_revision = before.revision.map(Id::to_bytes).transpose()?;
        let after_revision = after.revision.map(Id::to_bytes).transpose()?;
        let changed = tx.execute(
            "UPDATE selection SET root = ?1, generation = ?2, revision = ?3 WHERE singleton = 1 AND generation = ?4",
            params![after.root.to_bytes()?.as_slice(), next, after_revision.as_ref().map(|id| id.as_slice()), expected],
        )?;
        if changed != 1 {
            return Err(invalid("selection singleton/CAS invariant"));
        }
        let operation = mutation.operation();
        let onto = operation.rebase_onto().map(Id::to_bytes).transpose()?;
        let git_commit = operation.git_commit().map(|oid| oid.to_bytes());
        tx.execute(
            "INSERT INTO operations VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                request_key,
                operation.as_str(),
                before.root.to_bytes()?.as_slice(),
                expected,
                before_revision.as_ref().map(|id| id.as_slice()),
                after.root.to_bytes()?.as_slice(),
                next,
                after_revision.as_ref().map(|id| id.as_slice()),
                operation.undo_target(),
                onto.as_ref().map(|id| id.as_slice()),
                git_commit.as_ref().map(|oid| oid.as_slice())
            ],
        )?;
        tx.commit()?;
        Ok(Receipt {
            request_key: request_key.to_owned(),
            operation,
            before,
            after,
        })
    }

    fn reconcile(&self, prior: Receipt, expected: u64, mutation: Mutation<'_>) -> Result<Receipt> {
        if let Mutation::ImportGit { origin, .. } = mutation {
            return self.reconcile_git_import(prior, origin.commit);
        }
        if let Mutation::Rebase { onto, .. } = mutation {
            return self.reconcile_rebase(prior, expected, onto);
        }
        let same = prior.before.generation == expected
            && match (&prior.operation, mutation) {
                (Operation::ImportHistory, Mutation::ImportHistory { root, revision }) => {
                    prior.after.root == root && prior.after.revision == revision
                }
                (Operation::Source, Mutation::Source { root, .. }) => prior.after.root == root,
                (Operation::Undo { target: previous }, Mutation::Undo { target }) => {
                    previous == target
                }
                (
                    operation,
                    Mutation::Author {
                        author,
                        message,
                        amend,
                    },
                ) if *operation
                    == if amend {
                        Operation::Amend
                    } else {
                        Operation::Commit
                    } =>
                {
                    let id = prior
                        .after
                        .revision
                        .ok_or_else(|| invalid("commit receipt has no revision"))?;
                    let revision = history::read_revision(&self.connection, id)?;
                    revision.author == author
                        && revision.message == message
                        && revision.root == prior.after.root
                        && if amend {
                            let predecessor = history::read_revision(
                                &self.connection,
                                prior
                                    .before
                                    .revision
                                    .ok_or_else(|| invalid("amend receipt has no predecessor"))?,
                            )?;
                            revision.parents == predecessor.parents
                                && revision.change == predecessor.change
                        } else {
                            revision.parents
                                == prior.before.revision.into_iter().collect::<Vec<_>>()
                        }
                }
                _ => false,
            };
        if !same {
            return Err(Error::RequestKeyReuse(prior.request_key));
        }
        Ok(prior)
    }

    fn read_object(&self, id: Id) -> Result<Vec<u8>> {
        read_object_optional(&self.connection, id)?.ok_or(Error::MissingObject(id))
    }
}

const SCHEMA: &str = "
CREATE TABLE repository (singleton INTEGER PRIMARY KEY CHECK(singleton = 1), instance BLOB NOT NULL CHECK(length(instance) = 16)) STRICT;
CREATE TABLE objects (
    id BLOB PRIMARY KEY CHECK(length(id) = 33),
    length INTEGER NOT NULL CHECK(length BETWEEN 0 AND 16777216),
    payload BLOB NOT NULL CHECK(length(payload) = length)
) STRICT;
CREATE TABLE selection (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    root BLOB NOT NULL REFERENCES objects(id),
    generation INTEGER NOT NULL CHECK(generation >= 0),
    revision BLOB REFERENCES objects(id)
) STRICT;
CREATE TABLE git_origins (
    revision BLOB PRIMARY KEY REFERENCES objects(id),
    commit_oid BLOB NOT NULL UNIQUE CHECK(length(commit_oid) = 21 AND substr(commit_oid,1,1) = X'01'),
    tree_oid BLOB NOT NULL CHECK(length(tree_oid) = 21 AND substr(tree_oid,1,1) = X'01'),
    parents BLOB NOT NULL CHECK(length(parents) % 21 = 0 AND length(parents) <= 336),
    raw_commit BLOB NOT NULL CHECK(length(raw_commit) <= 16777216),
    checksum BLOB NOT NULL CHECK(length(checksum) = 32)
) STRICT;
CREATE TABLE git_exports (
    revision BLOB NOT NULL REFERENCES objects(id),
    algorithm INTEGER NOT NULL CHECK(algorithm = 1),
    policy INTEGER NOT NULL CHECK(policy = 1),
    commit_oid BLOB NOT NULL CHECK(length(commit_oid) = 21 AND substr(commit_oid,1,1) = X'01'),
    PRIMARY KEY(revision, algorithm, policy)
) STRICT;
CREATE TABLE operations (
    request_key TEXT PRIMARY KEY CHECK(length(CAST(request_key AS BLOB)) BETWEEN 1 AND 1024),
    kind TEXT NOT NULL CHECK(kind IN ('source', 'commit', 'amend', 'rebase', 'import-history', 'import-git', 'undo')),
    before_root BLOB NOT NULL REFERENCES objects(id),
    before_generation INTEGER NOT NULL CHECK(before_generation >= 0),
    before_revision BLOB REFERENCES objects(id),
    after_root BLOB NOT NULL REFERENCES objects(id),
    after_generation INTEGER NOT NULL CHECK(after_generation = before_generation + 1),
    after_revision BLOB REFERENCES objects(id),
    undo_target TEXT REFERENCES operations(request_key),
    onto_revision BLOB REFERENCES objects(id),
    git_commit BLOB REFERENCES git_origins(commit_oid),
    CHECK((kind = 'import-git' AND git_commit IS NOT NULL) OR (kind != 'import-git' AND git_commit IS NULL)),
    CHECK((kind = 'rebase' AND onto_revision IS NOT NULL) OR (kind != 'rebase' AND onto_revision IS NULL)),
    CHECK((kind = 'undo' AND undo_target IS NOT NULL) OR (kind != 'undo' AND undo_target IS NULL)),
    CHECK((kind = 'source' AND before_revision IS after_revision) OR
          (kind = 'commit' AND before_root = after_root AND after_revision IS NOT NULL AND before_revision IS NOT after_revision) OR
          (kind = 'amend' AND before_root = after_root AND before_revision IS NOT NULL AND after_revision IS NOT NULL AND before_revision IS NOT after_revision) OR
          (kind = 'import-history' AND before_generation = 0 AND before_revision IS NULL) OR
          (kind = 'import-git' AND before_generation = 0 AND before_revision IS NULL AND after_revision IS NOT NULL) OR
          (kind = 'rebase' AND before_revision IS NOT NULL AND after_revision IS NOT NULL) OR
          kind = 'undo')
) STRICT;
CREATE INDEX operations_git_anchor ON operations(after_revision) WHERE kind = 'import-git';
CREATE TRIGGER git_origins_no_update BEFORE UPDATE ON git_origins BEGIN SELECT RAISE(ABORT, 'immutable Git origin'); END;
CREATE TRIGGER git_origins_no_delete BEFORE DELETE ON git_origins BEGIN SELECT RAISE(ABORT, 'retained Git origin'); END;
CREATE TRIGGER git_exports_no_update BEFORE UPDATE ON git_exports BEGIN SELECT RAISE(ABORT, 'immutable Git mapping'); END;
CREATE TRIGGER git_exports_no_delete BEFORE DELETE ON git_exports BEGIN SELECT RAISE(ABORT, 'retained Git mapping'); END;
CREATE TRIGGER objects_no_update BEFORE UPDATE ON objects BEGIN SELECT RAISE(ABORT, 'immutable object'); END;
CREATE TRIGGER objects_no_delete BEFORE DELETE ON objects BEGIN SELECT RAISE(ABORT, 'retained object'); END;
CREATE TRIGGER operations_no_update BEFORE UPDATE ON operations BEGIN SELECT RAISE(ABORT, 'immutable receipt'); END;
CREATE TRIGGER operations_no_delete BEFORE DELETE ON operations BEGIN SELECT RAISE(ABORT, 'retained receipt'); END;
";

fn check_request_key(key: &str) -> Result<()> {
    if key.is_empty() || key.len() > MAX_REQUEST_BYTES {
        return Err(Error::Limit("request key must be 1..=1024 bytes"));
    }
    Ok(())
}
fn sync_repository_installation(directory: &Path, project: &Path) -> Result<()> {
    sync_directory(directory)?;
    sync_directory(project)
}

fn durability_fence(connection: &Connection) -> Result<()> {
    if !connection.is_autocommit() {
        return Err(Error::RecoveryRequired);
    }
    // SQLite owns all database/WAL descriptors. Opening and closing a separate
    // main-DB descriptor here would release its POSIX locks behind its back.
    // FULL syncs recovered WAL commits before backfill, then syncs the database.
    let (busy, log, checkpointed): (i64, i64, i64) =
        connection.query_row("PRAGMA main.wal_checkpoint(FULL)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
    if busy != 0 || log != checkpointed {
        return Err(Error::DurabilityPending);
    }
    if log < -1 {
        return Err(invalid("checkpoint frame counts"));
    }
    Ok(())
}

fn configure(connection: &Connection, initialize: bool) -> Result<()> {
    connection.busy_timeout(Duration::from_secs(5))?;
    // SQLite materializes result rows before Rust can inspect their declared
    // lengths. Bound that allocation too; allow record-header/ID overhead.
    let row_limit =
        i32::try_from(MAX_OBJECT_BYTES + 1024).map_err(|_| Error::Limit("SQLite row bytes"))?;
    connection.set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH, row_limit)?;
    if connection.limit(rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH)? != row_limit {
        return Err(invalid("SQLite row limit unavailable"));
    }
    if initialize {
        connection.pragma_update(None, "journal_mode", "WAL")?;
    }
    let mode: String = connection.pragma_query_value(None, "journal_mode", |r| r.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(invalid("SQLite WAL unavailable"));
    }
    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.pragma_update(None, "foreign_keys", true)?;
    #[cfg(target_os = "macos")]
    {
        connection.pragma_update(None, "fullfsync", true)?;
        connection.pragma_update(None, "checkpoint_fullfsync", true)?;
        let full: bool = connection.pragma_query_value(None, "fullfsync", |r| r.get(0))?;
        let checkpoint: bool =
            connection.pragma_query_value(None, "checkpoint_fullfsync", |r| r.get(0))?;
        if !full || !checkpoint {
            return Err(invalid("SQLite fullfsync unavailable"));
        }
    }
    let sync: i64 = connection.pragma_query_value(None, "synchronous", |r| r.get(0))?;
    let foreign: bool = connection.pragma_query_value(None, "foreign_keys", |r| r.get(0))?;
    if sync != 2 || !foreign {
        return Err(invalid("SQLite FULL/foreign_keys unavailable"));
    }
    Ok(())
}

fn require_source_root(root: Id) -> Result<()> {
    if !matches!(
        root.kind,
        crate::MAP_LEAF_KIND | crate::MAP_BRANCH_KIND | crate::CONFLICT_KIND
    ) {
        return Err(invalid(
            "requires a source Map or root Conflict, not a Revision or Blob",
        ));
    }
    Ok(())
}

type StoredSelection = (Vec<u8>, i64, Option<Vec<u8>>);

fn decode_selection((root, generation, revision): StoredSelection) -> Result<Selection> {
    let root = Id::from_bytes(&root)?;
    if !matches!(
        root.kind,
        crate::MAP_LEAF_KIND | crate::MAP_BRANCH_KIND | crate::CONFLICT_KIND
    ) {
        return Err(invalid("selection requires a source root"));
    }
    let revision = revision.map(|id| Id::from_bytes(&id)).transpose()?;
    if revision.is_some_and(|id| id.kind != crate::REVISION_KIND) {
        return Err(invalid("selection revision kind"));
    }
    Ok(Selection {
        root,
        revision,
        generation: generation
            .try_into()
            .map_err(|_| invalid("negative generation"))?,
    })
}
fn selection(connection: &Connection) -> Result<Selection> {
    decode_selection(connection.query_row(
        "SELECT root, generation, revision FROM selection WHERE singleton = 1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?)
}
fn receipt(connection: &Connection, key: &str) -> Result<Option<Receipt>> {
    type StoredReceipt = (
        String,
        StoredSelection,
        StoredSelection,
        Option<String>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
    );
    let row: Option<StoredReceipt> = connection.query_row(
        "SELECT kind, before_root, before_generation, before_revision, after_root, after_generation, after_revision, undo_target, onto_revision, git_commit FROM operations WHERE request_key = ?1",
        [key], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?, r.get(3)?), (r.get(4)?, r.get(5)?, r.get(6)?), r.get(7)?, r.get(8)?, r.get(9)?)),
    ).optional()?;
    row.map(|(kind, before, after, target, onto, git_commit)| {
        let before = decode_selection(before)?;
        let after = decode_selection(after)?;
        if before.generation.checked_add(1) != Some(after.generation) {
            return Err(invalid("receipt generation step"));
        }
        if (kind == "undo") != target.is_some() {
            return Err(invalid("receipt undo target"));
        }
        if (kind == "rebase") != onto.is_some() {
            return Err(invalid("receipt rebase target"));
        }
        if (kind == "import-git") != git_commit.is_some() {
            return Err(invalid("receipt Git commit"));
        }
        let operation = match kind.as_str() {
            "source" if before.revision == after.revision => Operation::Source,
            "commit"
                if before.root == after.root
                    && after.revision.is_some()
                    && before.revision != after.revision =>
            {
                Operation::Commit
            }
            "amend"
                if before.root == after.root
                    && before.revision.is_some()
                    && after.revision.is_some()
                    && before.revision != after.revision =>
            {
                Operation::Amend
            }
            "rebase" if before.revision.is_some() && after.revision.is_some() => {
                let onto = Id::from_bytes(&onto.ok_or_else(|| invalid("receipt rebase target"))?)?;
                if onto.kind != crate::REVISION_KIND {
                    return Err(invalid("receipt rebase target kind"));
                }
                Operation::Rebase { onto }
            }
            "undo" => {
                let target = target.ok_or_else(|| invalid("receipt undo target"))?;
                check_request_key(&target)?;
                Operation::Undo { target }
            }
            "import-history" if before.generation == 0 && before.revision.is_none() => {
                Operation::ImportHistory
            }
            "import-git"
                if before.generation == 0
                    && before.revision.is_none()
                    && after.revision.is_some() =>
            {
                Operation::ImportGit {
                    commit: crate::git::GitOid::from_bytes(
                        &git_commit.ok_or_else(|| invalid("receipt Git commit"))?,
                    )?,
                }
            }
            _ => return Err(invalid("receipt operation")),
        };
        Ok(Receipt {
            request_key: key.to_owned(),
            operation,
            before,
            after,
        })
    })
    .transpose()
}

fn insert_objects(
    connection: &Connection,
    records: impl IntoIterator<Item = (Id, Vec<u8>)>,
) -> Result<()> {
    for (id, payload) in records {
        // Do not accept corrupt dedup winners or overwrite immutable objects.
        if let Some(stored) = read_object_optional(connection, id)? {
            if stored != payload {
                return Err(invalid("object collision"));
            }
        } else {
            connection.execute(
                "INSERT INTO objects VALUES (?1, ?2, ?3)",
                params![id.to_bytes()?.as_slice(), payload.len() as i64, payload],
            )?;
        }
    }
    Ok(())
}

fn encode_bounded(object: &Object) -> Result<Vec<u8>> {
    if let Object::Blob(bytes) = object
        && bytes.len() > MAX_OBJECT_BYTES
    {
        return Err(Error::Limit("object bytes"));
    }
    let payload = object.encode()?;
    bounded_length(payload.len() as u64)?;
    Ok(payload)
}
fn bounded_length(length: u64) -> Result<usize> {
    if length > MAX_OBJECT_BYTES as u64 {
        return Err(Error::Limit("object bytes"));
    }
    Ok(length as usize)
}
fn budget(total: &mut usize, length: usize) -> Result<()> {
    *total = total
        .checked_add(length)
        .ok_or(Error::Limit("closure bytes"))?;
    if *total > MAX_CLOSURE_BYTES {
        return Err(Error::Limit("closure bytes"));
    }
    Ok(())
}

/// Walk only owning references, never Engine::objects() (which includes orphans).
fn collect_closure(
    roots: impl IntoIterator<Item = Id>,
    mut fetch: impl FnMut(Id) -> Result<Vec<u8>>,
) -> Result<BTreeMap<Id, Vec<u8>>> {
    let mut records = BTreeMap::new();
    let mut discovered: BTreeSet<_> = roots.into_iter().collect();
    let mut pending: Vec<_> = discovered.iter().copied().collect();
    let mut total = 0;
    while let Some(id) = pending.pop() {
        let payload = fetch(id)?;
        bounded_length(payload.len() as u64)?;
        budget(&mut total, payload.len())?;
        let object = Object::decode(id, &payload)?;
        for child in owning_refs(&object) {
            if discovered.insert(child) {
                if discovered.len() > MAX_OBJECTS {
                    return Err(Error::Limit("closure objects"));
                }
                pending.push(child);
            }
        }
        records.insert(id, payload);
    }
    Ok(records)
}
fn entry_ref(entry: &Entry) -> Option<Id> {
    match entry {
        Entry::File { blob, .. } | Entry::Symlink(blob) => Some(*blob),
        Entry::Directory(id) | Entry::Conflict(id) => Some(*id),
        Entry::Absent => None,
    }
}
fn owning_refs(object: &Object) -> Vec<Id> {
    match object {
        Object::Blob(_) => Vec::new(),
        Object::Map(Node::Leaf(entries)) => {
            entries.iter().filter_map(|(_, e)| entry_ref(e)).collect()
        }
        Object::Map(Node::Branch {
            terminal, children, ..
        }) => terminal
            .iter()
            .filter_map(entry_ref)
            .chain(children.values().copied())
            .collect(),
        Object::Conflict(c) => c
            .positive
            .iter()
            .chain(&c.negative)
            .filter_map(|(e, _)| entry_ref(e))
            .collect(),
        Object::Revision(r) => std::iter::once(r.root)
            .chain(r.parents.iter().copied())
            .collect(),
    }
}

fn read_object_optional(connection: &Connection, id: Id) -> Result<Option<Vec<u8>>> {
    let mut statement = connection.prepare("SELECT length, payload FROM objects WHERE id = ?1")?;
    let mut rows = statement.query([id.to_bytes()?.as_slice()])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let length: i64 = row.get(0)?;
    let length = bounded_length(u64::try_from(length).map_err(|_| invalid("object length"))?)?;
    let rusqlite::types::ValueRef::Blob(payload) = row.get_ref(1)? else {
        return Err(invalid("object payload storage type"));
    };
    if payload.len() != length {
        return Err(invalid("object payload length"));
    }
    // Retained objects must already own a complete closure. A missing dependency
    // is damaged storage, not a cache miss that admission may silently repair.
    let references = owning_refs(&Object::decode(id, payload)?);
    for reference in references {
        let exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM objects WHERE id = ?1)",
            [reference.to_bytes()?.as_slice()],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(Error::MissingObject(reference));
        }
    }
    Ok(Some(payload.to_vec()))
}
fn require_file(path: &Path) -> Result<()> {
    if !fs::symlink_metadata(path)?.file_type().is_file() {
        return Err(invalid("expected regular file, not symlink"));
    }
    Ok(())
}
fn require_directory(path: &Path) -> Result<()> {
    if !fs::symlink_metadata(path)?.file_type().is_dir() {
        return Err(invalid("expected directory, not symlink"));
    }
    Ok(())
}
fn sync_file(file: &File) -> Result<()> {
    file.sync_all()?;
    #[cfg(target_os = "macos")]
    rustix::fs::fcntl_fullfsync(file).map_err(std::io::Error::from)?;
    Ok(())
}
fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod fault_tests;

#[cfg(test)]
mod crash_tests {
    use super::*;
    use crate::engine::Edit;
    use std::process::Command;

    fn candidate() -> (Engine, Id) {
        let mut engine = Engine::new();
        let empty = engine.empty().unwrap();
        // Fixed fixture payload (matches fault_tests): identical bytes in every
        // storage variant, so only physical placement may differ.
        let blob = engine.blob(vec![7; 8192]).unwrap();
        let root = engine
            .edit(
                empty,
                &[Edit {
                    path: vec![b"payload".to_vec()],
                    expected: Entry::Absent,
                    value: Entry::File {
                        blob,
                        executable: false,
                    },
                }],
            )
            .unwrap();
        (engine, root)
    }

    #[test]
    fn request_preparation_keeps_one_snapshot_during_identical_commit() {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        use std::sync::mpsc;
        let project = tempfile::tempdir().unwrap();
        let repo = Repository::init(project.path()).unwrap();
        let before = repo.status().unwrap();
        let mut writer = Repository::open(project.path()).unwrap();
        let (engine, root) = candidate();
        let (start, waiting) = mpsc::channel();
        let (finished, done) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            waiting.recv().unwrap();
            let receipt = writer.commit_source(&engine, root, 0, "same").unwrap();
            finished.send(receipt.clone()).unwrap();
            receipt
        });
        let mut fired = false;
        repo.connection
            .authorizer(Some(move |context: AuthContext<'_>| {
                if !fired
                    && matches!(
                        context.action,
                        AuthAction::Read {
                            table_name: "selection",
                            ..
                        }
                    )
                {
                    fired = true;
                    start.send(()).unwrap();
                    done.recv().unwrap();
                }
                Authorization::Allow
            }))
            .unwrap();
        // Receipt absence is read first; authorizer pauses just before reading
        // selection, while the identical request commits on another connection.
        let selected = repo.preparation_base(0, "same").unwrap();
        assert_eq!(selected, before);
        let committed = worker.join().unwrap();
        assert_eq!(repo.receipt("same").unwrap().unwrap(), committed);
        assert_eq!(repo.preparation_base(0, "same").unwrap(), before);
    }

    // The only fault hooks are SQLite hooks on this test's private connection.
    // process::exit bypasses Connection destructors and SQLite close.
    #[test]
    fn crash_child() {
        let Some(path) = std::env::var_os("AVC_TEST_CRASH_PROJECT") else {
            return;
        };
        let phase = std::env::var("AVC_TEST_CRASH_PHASE").unwrap();
        let mut repo = Repository::open(Path::new(&path)).unwrap();
        if phase == "before" {
            repo.connection
                .commit_hook(Some(|| -> bool { std::process::exit(86) }))
                .unwrap();
        } else {
            repo.connection
                .wal_hook(Some(|_, _| std::process::exit(87)));
        }
        let (engine, root) = candidate();
        repo.commit_source(&engine, root, 0, "crash-request")
            .unwrap();
        panic!("fault hook did not terminate child");
    }

    #[test]
    fn process_crash_before_and_after_commit() {
        for (phase, code, generation) in [("before", 86, 0), ("after", 87, 1)] {
            let project = tempfile::tempdir().unwrap();
            let initial = Repository::init(project.path()).unwrap().status().unwrap();
            let status = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "repository::crash_tests::crash_child",
                    "--nocapture",
                ])
                .env("AVC_TEST_CRASH_PROJECT", project.path())
                .env("AVC_TEST_CRASH_PHASE", phase)
                .status()
                .unwrap();
            assert_eq!(status.code(), Some(code));
            let mut repo = Repository::open(project.path()).unwrap();
            let (selected, restored) = repo.load().unwrap();
            assert_eq!(selected.generation, generation);
            let (engine, root) = candidate();
            if generation == 0 {
                assert_eq!(selected, initial);
                assert!(repo.receipt("crash-request").unwrap().is_none());
                let count: i64 = repo
                    .connection
                    .query_row("SELECT count(*) FROM objects", [], |r| r.get(0))
                    .unwrap();
                assert_eq!(count, 1);
            } else {
                assert_eq!(selected.root, root);
                assert!(restored.read(root, &[b"payload".to_vec()]).is_ok());
                let prior = repo.receipt("crash-request").unwrap().unwrap();
                assert_eq!(
                    repo.commit_source(&Engine::new(), root, 0, "crash-request")
                        .unwrap(),
                    prior
                );
            }
            // Pre-commit payload inserts roll back; post-commit retry does not
            // advance the generation or repeat the effect.
            assert_eq!(
                repo.commit_source(&engine, root, 0, "crash-request")
                    .unwrap()
                    .after
                    .generation,
                1
            );
        }
    }
}
