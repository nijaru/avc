//! Experimental local storage, not a qualified source-custody implementation.
//!
//! SQLite WAL/FULL owns the selection and immutable receipts. Small canonical
//! payloads are inline; larger payloads are raw, one per immutable sealed file.
//! Compression, extents, large-object streaming, deletion and GC are deferred.
//! All closure operations buffer bounded data (and may hold several copies).
//! No leases are needed while locations and files are never removed or changed.
//!
//! Requires a trusted administration directory and a local filesystem honoring
//! SQLite locking, atomic hard links, file sync and directory sync. Sync errors
//! are fatal; OS/device power-loss behavior and hostile directory replacement
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
pub const INLINE_BYTES: usize = 1024;
const MAX_REQUEST_BYTES: usize = 1024;
const APPLICATION_ID: i64 = 0x41564331;
const SCHEMA_VERSION: i64 = 1;
const RAW_MAGIC: &[u8; 8] = b"AVCRAW01";
const EXPORT_MAGIC: &[u8; 8] = b"AVCEXP01";
const EXPORT_END: &[u8; 8] = b"AVCEND01";

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

/// A source-root-only local pin, not an authored revision. Authored revision
/// admission and selection are deferred; initialization creates no revision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Selection {
    pub root: Id,
    pub generation: u64,
}

/// Immutable private retention of both selections. Not exported by `export`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Receipt {
    pub request_key: String,
    pub before: Selection,
    pub after: Selection,
}

pub struct Repository {
    directory: PathBuf,
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
        fs::create_dir(directory.join("objects"))?;
        fs::create_dir(directory.join("objects/staged"))?;
        fs::create_dir(directory.join("objects/sealed"))?;
        sync_directory(&directory.join("objects/staged"))?;
        sync_directory(&directory.join("objects/sealed"))?;
        sync_directory(&directory.join("objects"))?;
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
            "INSERT INTO selection VALUES (1, ?1, 0)",
            [root.to_bytes()?.as_slice()],
        )?;
        tx.commit()?;
        sync_repository_installation(&directory, project_dir)?;
        Ok(Self {
            directory,
            connection,
        })
    }

    /// Opens only a recognized existing database; never creates a missing DB.
    pub fn open(project_dir: &Path) -> Result<Self> {
        let directory = project_dir.join(".avc");
        require_directory(&directory)?;
        require_directory(&directory.join("objects"))?;
        require_directory(&directory.join("objects/staged"))?;
        require_directory(&directory.join("objects/sealed"))?;
        require_file(&directory.join("store.sqlite"))?;
        let connection = Connection::open_with_flags(
            directory.join("store.sqlite"),
            OpenFlags::SQLITE_OPEN_READ_WRITE,
        )?;
        let app: i64 = connection.pragma_query_value(None, "application_id", |r| r.get(0))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if app != APPLICATION_ID || version != SCHEMA_VERSION {
            return Err(invalid("unknown database format"));
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
        let repo = Self {
            directory,
            connection,
        };
        repo.status()?;
        // A creator can fail or die after schema commit but before directory
        // installation is synced. Every opener completes that idempotent phase
        // before returning a handle that could acknowledge a new root.
        sync_repository_installation(&repo.directory, project_dir)?;
        Ok(repo)
    }

    /// Metadata only: does not assert physical availability or source custody.
    pub fn status(&self) -> Result<Selection> {
        selection(&self.connection)
    }

    pub fn receipt(&self, request_key: &str) -> Result<Option<Receipt>> {
        receipt(&self.connection, request_key)
    }

    /// Resolve the original source base for a conditional request/retry. Receipt
    /// and selection are one SQLite read snapshot: an identical concurrent
    /// commit cannot turn a valid preparation into a spurious Stale result.
    pub fn preparation_base(
        &self,
        expected_generation: u64,
        request_key: &str,
    ) -> Result<Selection> {
        check_request_key(request_key)?;
        let tx = self.connection.unchecked_transaction()?;
        let previous = match receipt(&tx, request_key)? {
            Some(prior) => {
                if prior.before.generation != expected_generation {
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
        let records = collect_closure(root, |id| self.read_object(id))?;
        let mut engine = Engine::new();
        engine.import(records, root, limits())?;
        Ok(engine)
    }

    /// Durably admits only the exact owning closure, then atomically advances
    /// the selection and retains before/after roots in a receipt. Request identity
    /// is `(request_key, root, expected_generation)`. A committed retry returns
    /// its original receipt before stale checks or consulting the supplied engine.
    /// A stale/failed preparation may leave unreachable sealed/staged files.
    pub fn commit_source(
        &mut self,
        engine: &Engine,
        root: Id,
        expected_generation: u64,
        request_key: &str,
    ) -> Result<Receipt> {
        self.commit(engine, root, expected_generation, request_key, false)
    }

    fn commit(
        &mut self,
        engine: &Engine,
        root: Id,
        expected_generation: u64,
        request_key: &str,
        empty_only: bool,
    ) -> Result<Receipt> {
        check_request_key(request_key)?;
        if let Some(prior) = self.receipt(request_key)? {
            return same_request(prior, root, expected_generation);
        }
        if !matches!(
            root.kind,
            crate::MAP_LEAF_KIND | crate::MAP_BRANCH_KIND | crate::CONFLICT_KIND
        ) {
            return Err(invalid(
                "selection requires a source root, not a revision or blob",
            ));
        }
        let expected =
            i64::try_from(expected_generation).map_err(|_| Error::Limit("generation"))?;
        // Refuse known stale work before preparing data. Recheck under the write
        // transaction, including a receipt created by a concurrent same request.
        let current = self.status()?;
        if current.generation != expected_generation || (empty_only && current.generation != 0) {
            // A concurrent identical request may have committed between the
            // first receipt lookup and the selection read.
            if let Some(prior) = self.receipt(request_key)? {
                return same_request(prior, root, expected_generation);
            }
            if empty_only {
                return Err(Error::NotEmpty);
            }
            return Err(Error::Stale {
                expected: expected_generation,
                actual: current.generation,
            });
        }
        let records = collect_closure(root, |id| encode_bounded(engine.object(id)?))?;
        engine.verify(root, limits())?;
        let mut prepared = Vec::new();
        for (id, payload) in records {
            if let Some(stored) = read_object_optional(&self.connection, &self.directory, id)? {
                if stored != payload {
                    return Err(invalid("object collision"));
                }
            } else {
                if payload.len() > INLINE_BYTES {
                    install(&self.directory, id, &payload)?;
                }
                prepared.push((id, payload));
            }
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(prior) = receipt(&tx, request_key)? {
            return same_request(prior, root, expected_generation);
        }
        let before = selection(&tx)?;
        if empty_only && before.generation != 0 {
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
        for (id, payload) in prepared {
            // Recheck concurrent dedup winners rather than accepting a corrupt
            // existing row with INSERT OR IGNORE.
            if let Some(stored) = read_object_optional(&tx, &self.directory, id)? {
                if stored != payload {
                    return Err(invalid("object collision"));
                }
            } else {
                let inline = (payload.len() <= INLINE_BYTES).then_some(payload.as_slice());
                tx.execute(
                    "INSERT INTO objects VALUES (?1, ?2, ?3)",
                    params![id.to_bytes()?.as_slice(), payload.len() as i64, inline],
                )?;
            }
        }
        let changed = tx.execute("UPDATE selection SET root = ?1, generation = ?2 WHERE singleton = 1 AND generation = ?3", params![root.to_bytes()?.as_slice(), next, expected])?;
        if changed != 1 {
            return Err(invalid("selection singleton/CAS invariant"));
        }
        tx.execute(
            "INSERT INTO operations VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                request_key,
                before.root.to_bytes()?.as_slice(),
                expected,
                root.to_bytes()?.as_slice(),
                next
            ],
        )?;
        tx.commit()?;
        Ok(Receipt {
            request_key: request_key.to_owned(),
            before,
            after: Selection {
                root,
                generation: next as u64,
            },
        })
    }

    fn read_object(&self, id: Id) -> Result<Vec<u8>> {
        read_object_optional(&self.connection, &self.directory, id)?.ok_or(Error::MissingObject(id))
    }

    /// Independent object-only v1 binary stream: explicit root and exact sorted
    /// owning records, followed by a completion marker and BLAKE3 of all preceding
    /// bytes. No private receipts, local selections or orphan objects are included.
    /// Failure can leave a partial output; only the complete stream is importable.
    /// The caller owns output flush/sync and atomic publication, if required.
    pub fn export(&self, root: Id, mut writer: impl Write) -> Result<()> {
        let engine = self.load_root(root)?;
        let records = collect_closure(root, |id| encode_bounded(engine.object(id)?))?;
        let mut hash = blake3::Hasher::new();
        write_hashed(&mut writer, &mut hash, EXPORT_MAGIC)?;
        write_hashed(&mut writer, &mut hash, &root.to_bytes()?)?;
        write_hashed(
            &mut writer,
            &mut hash,
            &(records.len() as u64).to_le_bytes(),
        )?;
        for (id, payload) in records {
            write_hashed(&mut writer, &mut hash, &id.to_bytes()?)?;
            write_hashed(
                &mut writer,
                &mut hash,
                &(payload.len() as u64).to_le_bytes(),
            )?;
            write_hashed(&mut writer, &mut hash, &payload)?;
        }
        write_hashed(&mut writer, &mut hash, EXPORT_END)?;
        writer.write_all(hash.finalize().as_bytes())?;
        Ok(())
    }

    /// Verifies each typed hash, completed EOF/hash, exact closure and Engine
    /// invariants before any admission. Initial selection must still be generation
    /// zero at commit. A successful same-key retry returns its original receipt.
    /// The reader must terminate (this is an object stream, not a network frame).
    pub fn import(&mut self, mut reader: impl Read, request_key: &str) -> Result<Receipt> {
        let mut hash = blake3::Hasher::new();
        if read_hashed::<8>(&mut reader, &mut hash)? != *EXPORT_MAGIC {
            return Err(invalid("export version/magic"));
        }
        let root = Id::from_bytes(&read_hashed::<33>(&mut reader, &mut hash)?)?;
        let count = u64::from_le_bytes(read_hashed::<8>(&mut reader, &mut hash)?);
        if count == 0 || count > MAX_OBJECTS as u64 {
            return Err(Error::Limit("export object count"));
        }
        let mut records = Vec::new();
        let mut previous = None;
        let mut total = 0;
        for _ in 0..count {
            let id = Id::from_bytes(&read_hashed::<33>(&mut reader, &mut hash)?)?;
            if previous.is_some_and(|p| p >= id) {
                return Err(invalid("export records must be unique and sorted"));
            }
            previous = Some(id);
            let length = u64::from_le_bytes(read_hashed::<8>(&mut reader, &mut hash)?);
            let length = bounded_length(length)?;
            budget(&mut total, length)?;
            let mut payload = vec![0; length];
            reader.read_exact(&mut payload)?;
            hash.update(&payload);
            Object::decode(id, &payload)?;
            records.push((id, payload));
        }
        if read_hashed::<8>(&mut reader, &mut hash)? != *EXPORT_END {
            return Err(invalid("incomplete export"));
        }
        let mut digest = [0; 32];
        reader.read_exact(&mut digest)?;
        if digest != *hash.finalize().as_bytes() {
            return Err(invalid("export checksum"));
        }
        require_eof(&mut reader)?;
        let mut engine = Engine::new();
        engine.import(records, root, limits())?;
        let exact = collect_closure(root, |id| encode_bounded(engine.object(id)?))?;
        if exact.len() as u64 != count {
            return Err(invalid("export contains non-owning objects"));
        }
        self.commit(&engine, root, 0, request_key, true)
    }
}

const SCHEMA: &str = "
CREATE TABLE repository (singleton INTEGER PRIMARY KEY CHECK(singleton = 1), instance BLOB NOT NULL CHECK(length(instance) = 16)) STRICT;
CREATE TABLE objects (
    id BLOB PRIMARY KEY CHECK(length(id) = 33),
    length INTEGER NOT NULL CHECK(length BETWEEN 0 AND 16777216),
    payload BLOB,
    CHECK((length <= 1024 AND payload IS NOT NULL AND length(payload) = length) OR (length > 1024 AND payload IS NULL))
) STRICT;
CREATE TABLE selection (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    root BLOB NOT NULL REFERENCES objects(id),
    generation INTEGER NOT NULL CHECK(generation >= 0)
) STRICT;
CREATE TABLE operations (
    request_key TEXT PRIMARY KEY CHECK(length(CAST(request_key AS BLOB)) BETWEEN 1 AND 1024),
    before_root BLOB NOT NULL REFERENCES objects(id),
    before_generation INTEGER NOT NULL CHECK(before_generation >= 0),
    after_root BLOB NOT NULL REFERENCES objects(id),
    after_generation INTEGER NOT NULL CHECK(after_generation = before_generation + 1)
) STRICT;
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
    for suffix in ["objects/staged", "objects/sealed", "objects", ""] {
        sync_directory(&directory.join(suffix))?;
    }
    sync_directory(project)?;
    sync_file(&File::open(directory.join("store.sqlite"))?)
}

fn configure(connection: &Connection, initialize: bool) -> Result<()> {
    connection.busy_timeout(Duration::from_secs(5))?;
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

fn selection(connection: &Connection) -> Result<Selection> {
    let (root, generation): (Vec<u8>, i64) = connection.query_row(
        "SELECT root, generation FROM selection WHERE singleton = 1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(Selection {
        root: Id::from_bytes(&root)?,
        generation: generation
            .try_into()
            .map_err(|_| invalid("negative generation"))?,
    })
}
fn receipt(connection: &Connection, key: &str) -> Result<Option<Receipt>> {
    let row: Option<(Vec<u8>, i64, Vec<u8>, i64)> = connection.query_row("SELECT before_root, before_generation, after_root, after_generation FROM operations WHERE request_key = ?1", [key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).optional()?;
    row.map(|(before, bg, after, ag)| {
        let before_generation = u64::try_from(bg).map_err(|_| invalid("receipt generation"))?;
        let after_generation = u64::try_from(ag).map_err(|_| invalid("receipt generation"))?;
        if before_generation.checked_add(1) != Some(after_generation) {
            return Err(invalid("receipt generation step"));
        }
        Ok(Receipt {
            request_key: key.to_owned(),
            before: Selection {
                root: Id::from_bytes(&before)?,
                generation: before_generation,
            },
            after: Selection {
                root: Id::from_bytes(&after)?,
                generation: after_generation,
            },
        })
    })
    .transpose()
}
fn same_request(prior: Receipt, root: Id, expected: u64) -> Result<Receipt> {
    if prior.after.root != root || prior.before.generation != expected {
        return Err(Error::RequestKeyReuse(prior.request_key));
    }
    Ok(prior)
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
    root: Id,
    mut fetch: impl FnMut(Id) -> Result<Vec<u8>>,
) -> Result<BTreeMap<Id, Vec<u8>>> {
    let mut records = BTreeMap::new();
    let mut discovered = BTreeSet::from([root]);
    let mut pending = vec![root];
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

fn read_object_optional(
    connection: &Connection,
    directory: &Path,
    id: Id,
) -> Result<Option<Vec<u8>>> {
    let mut statement = connection.prepare("SELECT length, payload FROM objects WHERE id = ?1")?;
    let mut rows = statement.query([id.to_bytes()?.as_slice()])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let length: i64 = row.get(0)?;
    let length = bounded_length(u64::try_from(length).map_err(|_| invalid("object length"))?)?;
    let payload = match row.get_ref(1)? {
        rusqlite::types::ValueRef::Blob(payload) => {
            if length > INLINE_BYTES || payload.len() != length {
                return Err(invalid("inline length/location"));
            }
            payload.to_vec()
        }
        rusqlite::types::ValueRef::Null => {
            if length <= INLINE_BYTES {
                return Err(invalid("sealed length/location"));
            }
            read_sealed(
                &directory.join("objects/sealed").join(id.to_string()),
                id,
                length,
            )?
        }
        _ => return Err(invalid("object payload storage type")),
    };
    Object::decode(id, &payload)?;
    Ok(Some(payload))
}
fn read_sealed(path: &Path, id: Id, length: usize) -> Result<Vec<u8>> {
    require_file(path)?;
    let mut file = File::open(path)?;
    if file.metadata()?.len() != (48 + length) as u64 {
        return Err(invalid("sealed file length"));
    }
    let mut header = [0; 48];
    file.read_exact(&mut header)?;
    if &header[..8] != RAW_MAGIC || header[8..16] != (length as u64).to_le_bytes() {
        return Err(invalid("sealed version/codec/length"));
    }
    let mut payload = vec![0; length];
    file.read_exact(&mut payload)?;
    require_eof(&mut file)?;
    if header[16..48] != *blake3::hash(&payload).as_bytes() {
        return Err(invalid("sealed checksum"));
    }
    Object::decode(id, &payload)?;
    Ok(payload)
}
fn install(directory: &Path, id: Id, payload: &[u8]) -> Result<()> {
    let sealed = directory.join("objects/sealed");
    let final_path = sealed.join(id.to_string());
    let mut staged = tempfile::NamedTempFile::new_in(directory.join("objects/staged"))?;
    staged.write_all(RAW_MAGIC)?;
    staged.write_all(&(payload.len() as u64).to_le_bytes())?;
    staged.write_all(blake3::hash(payload).as_bytes())?;
    staged.write_all(payload)?;
    sync_file(staged.as_file())?;
    // Atomic no-clobber installation. Competing writers can only publish complete
    // files; a leftover final name must verify, never be overwritten/repaired.
    match fs::hard_link(staged.path(), &final_path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    if read_sealed(&final_path, id, payload.len())? != payload {
        return Err(invalid("sealed collision"));
    }
    // Even a dedup winner may not have synced its new directory entry yet.
    sync_file(&File::open(&final_path)?)?;
    sync_directory(&sealed)?;
    sync_file(&File::open(&final_path)?)?;
    Ok(())
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

fn write_hashed(writer: &mut impl Write, hash: &mut blake3::Hasher, bytes: &[u8]) -> Result<()> {
    writer.write_all(bytes)?;
    hash.update(bytes);
    Ok(())
}
fn read_hashed<const N: usize>(
    reader: &mut impl Read,
    hash: &mut blake3::Hasher,
) -> Result<[u8; N]> {
    let mut bytes = [0; N];
    reader.read_exact(&mut bytes)?;
    hash.update(&bytes);
    Ok(bytes)
}
fn require_eof(reader: &mut impl Read) -> Result<()> {
    let mut extra = [0];
    loop {
        match reader.read(&mut extra) {
            Ok(0) => return Ok(()),
            Ok(_) => return Err(invalid("trailing bytes")),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
}

#[cfg(test)]
mod crash_tests {
    use super::*;
    use crate::engine::Edit;
    use std::process::Command;

    fn candidate() -> (Engine, Id) {
        let mut engine = Engine::new();
        let empty = engine.empty().unwrap();
        let blob = engine.blob(vec![7; INLINE_BYTES + 1]).unwrap();
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
    // process::exit bypasses Connection/TempFile destructors and SQLite close.
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
                assert_eq!(
                    fs::read_dir(project.path().join(".avc/objects/sealed"))
                        .unwrap()
                        .count(),
                    1
                );
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
            // Pre-commit orphan installation is reusable; post-commit retry does
            // not advance the generation or repeat the effect.
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
