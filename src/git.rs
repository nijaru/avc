//! Bounded, local SHA-1 Git translation. No refs, index, checkout, or network.
//!
//! Limits apply independently to Git payloads and native canonical payloads:
//! 16 MiB/object, 256 MiB/closure, 100,000 unique objects and expanded entries,
//! 256 path components and 4096 path bytes. Export policy 1 refuses empty
//! directory entries (not an empty root), conflicts, negative dates and
//! NUL-containing commit messages.
//! Git's own object validator also refuses `.git`/HFS/NTFS aliases and other
//! Git-invalid tree names/modes before import or candidate writes.
//! Git administration files are locally trusted;
//! inherited environment, configuration-driven execution, and source bytes are not.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::str::FromStr;

use crate::engine::{Engine, Limits};
use crate::repository::{Error, MAX_CLOSURE_BYTES, MAX_OBJECT_BYTES, MAX_OBJECTS, Result};
use crate::{Entry, Id, Object, Revision};

const MAX_ENTRIES: usize = 100_000;
const MAX_AUTHOR: usize = 4096;
const MAX_MESSAGE: usize = 1024 * 1024;

#[cfg(test)]
mod tests;

/// Persisted OIDs always include algorithm tag 1, never an untyped digest.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct GitOid([u8; 20]);

impl GitOid {
    pub fn to_bytes(self) -> [u8; 21] {
        let mut bytes = [1; 21];
        bytes[1..].copy_from_slice(&self.0);
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != 21 || bytes[0] != 1 {
            return Err(invalid("Git OID requires SHA-1 algorithm tag and 20 bytes"));
        }
        let mut digest = [0; 20];
        digest.copy_from_slice(&bytes[1..]);
        Ok(Self(digest))
    }

    fn hex(self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

impl fmt::Display for GitOid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sha1:{}", self.hex())
    }
}

impl FromStr for GitOid {
    type Err = Error;
    fn from_str(text: &str) -> Result<Self> {
        let text = text.strip_prefix("sha1:").unwrap_or(text);
        if text.len() != 40 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid("expected a SHA-1 Git OID"));
        }
        let mut digest = [0; 20];
        for (out, pair) in digest.iter_mut().zip(text.as_bytes().as_chunks::<2>().0) {
            let digit = |b: u8| {
                if b <= b'9' {
                    b - b'0'
                } else {
                    b.to_ascii_lowercase() - b'a' + 10
                }
            };
            *out = digit(pair[0]) * 16 + digit(pair[1]);
        }
        Ok(Self(digest))
    }
}

#[derive(Debug)]
pub struct GitCommit {
    pub oid: GitOid,
    pub tree: GitOid,
    pub parents: Vec<GitOid>,
    pub raw: Vec<u8>,
    pub author: String,
    pub message: String,
    pub time_ns: i64,
    pub timezone_minutes: i16,
}

pub struct GitStore {
    git_dir: PathBuf,
}

impl GitStore {
    /// Explicit administration directory only; never discovers a parent repo.
    pub fn open(git_dir: &Path) -> Result<Self> {
        let git_dir = git_dir.canonicalize()?;
        if !git_dir.is_dir() {
            return Err(invalid("Git administration path is not a directory"));
        }
        let store = Self { git_dir };
        let format = store.small(&["rev-parse", "--show-object-format=storage"], &[], 64)?;
        if format != b"sha1\n" {
            return Err(invalid("only SHA-1 Git repositories are supported"));
        }
        Ok(store)
    }

    fn command(&self) -> Command {
        let mut command = Command::new("git");
        // Raw cat-file never applies filters/textconv. Empty allow-list also
        // overrides per-protocol config, including custom remote helpers.
        command.env_clear();
        if let Some(path) = std::env::var_os("PATH") {
            command.env("PATH", path);
        }
        command
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_NO_REPLACE_OBJECTS", "1")
            .env("GIT_GRAFT_FILE", "/dev/null")
            .env("GIT_NO_LAZY_FETCH", "1")
            .env("GIT_ALLOW_PROTOCOL", "")
            .env("GIT_PROTOCOL_FROM_USER", "0")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("LC_ALL", "C")
            .arg("--no-pager")
            .arg("--no-replace-objects")
            .arg("--no-lazy-fetch")
            .arg("--git-dir")
            .arg(&self.git_dir);
        for setting in [
            "core.hooksPath=/dev/null",
            "core.fsmonitor=false",
            "core.useReplaceRefs=false",
            "core.commitGraph=false",
            "protocol.allow=never",
            "gc.auto=0",
            "maintenance.auto=false",
        ] {
            command.args(["-c", setting]);
        }
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        command
    }

    fn run<T>(
        &self,
        args: &[&str],
        input: &[u8],
        read: impl FnOnce(&mut BufReader<std::process::ChildStdout>) -> Result<T>,
    ) -> Result<T> {
        let mut child = Reap(self.command().args(args).spawn()?);
        let mut stdin = child
            .0
            .stdin
            .take()
            .ok_or_else(|| invalid("Git stdin unavailable"))?;
        // Only hash-object receives a large input. Its output is one OID after
        // consuming stdin; cat-file receives exactly one 41-byte request.
        stdin.write_all(input)?;
        drop(stdin);
        let stdout = child
            .0
            .stdout
            .take()
            .ok_or_else(|| invalid("Git stdout unavailable"))?;
        let value = read(&mut BufReader::new(stdout))?;
        if !child.0.wait()?.success() {
            return Err(invalid("Git plumbing subprocess failed (stderr discarded)"));
        }
        Ok(value)
    }

    fn small(&self, args: &[&str], input: &[u8], cap: usize) -> Result<Vec<u8>> {
        self.run(args, input, |reader| {
            let mut output = Vec::new();
            reader.take((cap + 1) as u64).read_to_end(&mut output)?;
            if output.len() > cap {
                return Err(Error::Limit("Git command output"));
            }
            Ok(output)
        })
    }

    fn hash(&self, kind: Kind, data: &[u8], write: bool) -> Result<GitOid> {
        if data.len() > MAX_OBJECT_BYTES {
            return Err(Error::Limit("Git object payload"));
        }
        let mut args = vec!["hash-object", "-t", kind.name(), "--stdin", "--no-filters"];
        // Keep Git's object-grammar checks in both read-only preflight and
        // writes. This does not require or fetch historical commit parents.
        if write {
            args.push("-w");
        }
        let output = self.small(&args, data, 64)?;
        let text = std::str::from_utf8(&output).map_err(|_| invalid("non-ASCII Git hash"))?;
        if text.len() != 41 || !text.ends_with('\n') {
            return Err(invalid("invalid Git hash output"));
        }
        text[..40].parse()
    }

    fn object(&self, oid: GitOid, expected: Kind, remaining: usize) -> Result<Vec<u8>> {
        let raw = self.run(
            &["cat-file", "--batch", "--no-use-mailmap"],
            format!("{}\n", oid.hex()).as_bytes(),
            |reader| {
                let mut header = Vec::new();
                reader.by_ref().take(129).read_until(b'\n', &mut header)?;
                if header.len() > 128 || !header.ends_with(b"\n") {
                    return Err(invalid("invalid or overlong Git object header"));
                }
                let header = std::str::from_utf8(&header)
                    .map_err(|_| invalid("invalid Git object header"))?;
                let fields: Vec<_> = header.trim_end_matches('\n').split(' ').collect();
                if fields.len() != 3 || fields[0] != oid.hex() || fields[1] != expected.name() {
                    return Err(invalid("missing Git object or incorrect object type"));
                }
                if fields[2].is_empty() || !fields[2].bytes().all(|b| b.is_ascii_digit()) {
                    return Err(invalid("invalid Git object size"));
                }
                let length: usize = fields[2]
                    .parse()
                    .map_err(|_| Error::Limit("Git object size"))?;
                if length > MAX_OBJECT_BYTES || length > remaining {
                    return Err(Error::Limit("Git payload budget"));
                }
                let mut data = vec![0; length];
                reader.read_exact(&mut data)?;
                let mut end = [0];
                reader.read_exact(&mut end)?;
                if end != *b"\n" || reader.read(&mut end)? != 0 {
                    return Err(invalid("Git payload framing"));
                }
                Ok(data)
            },
        )?;
        // cat-file returns bytes from an OID slot, not a verified content hash.
        // Normal hash-object also applies Git's object-grammar checks (including
        // filesystem aliases of .git), without fetching or traversing children.
        if self.hash(expected, &raw, false)? != oid {
            return Err(invalid("Git object hash mismatch"));
        }
        Ok(raw)
    }

    pub fn read_commit(&self, oid: GitOid) -> Result<GitCommit> {
        parse_commit(oid, self.object(oid, Kind::Commit, MAX_OBJECT_BYTES)?)
    }

    pub fn import_source(&self, tree: GitOid, engine: &mut Engine) -> Result<Id> {
        let mut import = Import {
            store: self,
            native: Engine::new(),
            entries: 0,
            git_budget: Budget::default(),
            trees: BTreeMap::new(),
            blobs: BTreeMap::new(),
            active: BTreeSet::new(),
        };
        let (root, _) = import.tree(tree, 0)?;
        // One final pass budgets the serialized native closure. The scratch
        // engine is bounded by the verified Git byte/object/entry budgets, but
        // radix expansion may exceed the native budget and must not leak into
        // the caller's authority. Bulk construction retains no edit orphans.
        let mut budget = Budget::default();
        let mut records = Vec::new();
        for (id, object) in import.native.objects() {
            let raw = object.encode()?;
            budget.add(raw.len())?;
            records.push((id, raw));
        }
        engine.import(records, root, limits())?;
        Ok(root)
    }
}

/// Drop handles every short read, parse failure, broken pipe and wait error.
/// Stderr goes to the null device rather than a bounded pipe that can deadlock.
struct Reap(Child);
impl Drop for Reap {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    Blob,
    Tree,
    Commit,
}
impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Blob => "blob",
            Self::Tree => "tree",
            Self::Commit => "commit",
        }
    }
}

#[derive(Default)]
struct Budget {
    bytes: usize,
    objects: usize,
}
impl Budget {
    fn add(&mut self, length: usize) -> Result<()> {
        if length > MAX_OBJECT_BYTES {
            return Err(Error::Limit("object payload"));
        }
        if self.objects >= MAX_OBJECTS || length > MAX_CLOSURE_BYTES - self.bytes {
            return Err(Error::Limit("Git/native closure"));
        }
        self.objects += 1;
        self.bytes += length;
        Ok(())
    }
}

// Metrics count the expanded logical tree, not just unique DAG nodes. This
// prevents tiny shared subtrees from bypassing traversal and path budgets.
#[derive(Clone, Copy, Default)]
struct Bounds {
    entries: usize,
    depth: usize,
    path: usize,
}
impl Bounds {
    fn entry(&mut self, name: &[u8], child: Self) -> Result<()> {
        self.entries = self
            .entries
            .checked_add(1 + child.entries)
            .ok_or(Error::Limit("expanded Git entries"))?;
        self.depth = self.depth.max(1 + child.depth);
        self.path = self
            .path
            .max(name.len() + usize::from(child.depth != 0) + child.path);
        if self.entries > MAX_ENTRIES || self.depth > 256 || self.path > 4096 {
            return Err(Error::Limit("expanded Git entries/depth/path"));
        }
        Ok(())
    }
}

struct Import<'a> {
    store: &'a GitStore,
    native: Engine,
    entries: usize,
    git_budget: Budget,
    trees: BTreeMap<GitOid, (Id, Bounds)>,
    blobs: BTreeMap<GitOid, Id>,
    active: BTreeSet<GitOid>,
}
impl Import<'_> {
    fn blob(&mut self, oid: GitOid) -> Result<Id> {
        if let Some(id) = self.blobs.get(&oid) {
            return Ok(*id);
        }
        if self.git_budget.objects >= MAX_OBJECTS {
            return Err(Error::Limit("Git closure objects"));
        }
        let data = self
            .store
            .object(oid, Kind::Blob, MAX_CLOSURE_BYTES - self.git_budget.bytes)?;
        self.git_budget.add(data.len())?;
        let id = self.native.blob(data)?;
        self.blobs.insert(oid, id);
        Ok(id)
    }
    fn tree(&mut self, oid: GitOid, depth: usize) -> Result<(Id, Bounds)> {
        if depth > 256 {
            return Err(Error::Limit("Git tree nesting"));
        }
        if let Some(result) = self.trees.get(&oid) {
            return Ok(*result);
        }
        if !self.active.insert(oid) {
            return Err(invalid("cyclic Git tree"));
        }
        if self.git_budget.objects >= MAX_OBJECTS {
            return Err(Error::Limit("Git closure objects"));
        }
        let raw = self
            .store
            .object(oid, Kind::Tree, MAX_CLOSURE_BYTES - self.git_budget.bytes)?;
        self.git_budget.add(raw.len())?;
        let entries = parse_tree(&raw)?;
        self.entries += entries.len();
        if self.entries > MAX_ENTRIES {
            return Err(Error::Limit("Git closure bindings"));
        }
        let mut bindings = Vec::with_capacity(entries.len());
        let mut bounds = Bounds::default();
        for (name, mode, child) in entries {
            let (entry, child_bounds) = match mode {
                Mode::Directory => {
                    let (id, bounds) = self.tree(child, depth + 1)?;
                    (Entry::Directory(id), bounds)
                }
                Mode::File | Mode::Executable => (
                    Entry::File {
                        blob: self.blob(child)?,
                        executable: mode == Mode::Executable,
                    },
                    Bounds::default(),
                ),
                Mode::Symlink => (Entry::Symlink(self.blob(child)?), Bounds::default()),
            };
            bounds.entry(&name, child_bounds)?;
            bindings.push((name, entry));
        }
        let id = self.native.directory(bindings)?;
        self.active.remove(&oid);
        self.trees.insert(oid, (id, bounds));
        Ok((id, bounds))
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Mode {
    Directory,
    File,
    Executable,
    Symlink,
}
impl Mode {
    fn bytes(self) -> &'static [u8] {
        match self {
            Self::Directory => b"40000",
            Self::File => b"100644",
            Self::Executable => b"100755",
            Self::Symlink => b"120000",
        }
    }
}
fn sort_key(name: &[u8], mode: Mode) -> Vec<u8> {
    let mut key = name.to_vec();
    if mode == Mode::Directory {
        key.push(b'/');
    }
    key
}
fn parse_tree(mut raw: &[u8]) -> Result<Vec<(Vec<u8>, Mode, GitOid)>> {
    let mut entries = Vec::new();
    let mut names = BTreeSet::new();
    let mut previous = None;
    while !raw.is_empty() {
        if entries.len() >= MAX_ENTRIES {
            return Err(Error::Limit("Git tree entries"));
        }
        let space = raw
            .iter()
            .position(|b| *b == b' ')
            .ok_or_else(|| invalid("Git tree mode framing"))?;
        let mode = match &raw[..space] {
            b"40000" => Mode::Directory,
            b"100644" => Mode::File,
            b"100755" => Mode::Executable,
            b"120000" => Mode::Symlink,
            _ => return Err(invalid("unsupported Git mode (including gitlinks)")),
        };
        raw = &raw[space + 1..];
        let nul = raw
            .iter()
            .position(|b| *b == 0)
            .ok_or_else(|| invalid("Git tree name framing"))?;
        let name = &raw[..nul];
        crate::validate_name(name)?;
        if !names.insert(name.to_vec()) {
            return Err(invalid("duplicate Git basename"));
        }
        let key = sort_key(name, mode);
        if previous.as_ref().is_some_and(|previous| previous >= &key) {
            return Err(invalid("unsorted Git tree"));
        }
        previous = Some(key);
        let digest = raw
            .get(nul + 1..nul + 21)
            .ok_or_else(|| invalid("truncated Git tree OID"))?;
        let mut oid = [0; 20];
        oid.copy_from_slice(digest);
        entries.push((name.to_vec(), mode, GitOid(oid)));
        raw = &raw[nul + 21..];
    }
    Ok(entries)
}

/// Read-only, bounded preflight followed by immutable candidate-object writes.
/// A failed preflight invalidates the plan: `write` refuses even objects added
/// by earlier calls. A successful write never publishes a ref.
#[derive(Default)]
pub struct GitPlan {
    objects: BTreeMap<GitOid, (Kind, Vec<u8>)>,
    roots: BTreeMap<Id, (GitOid, Bounds)>,
    blobs: BTreeMap<Id, GitOid>,
    budget: Budget,
    failed: bool,
}
impl GitPlan {
    pub fn new() -> Self {
        Self::default()
    }

    fn add(&mut self, store: &GitStore, kind: Kind, raw: Vec<u8>) -> Result<GitOid> {
        let oid = store.hash(kind, &raw, false)?;
        if let Some((previous_kind, previous)) = self.objects.get(&oid) {
            if *previous_kind != kind || previous != &raw {
                return Err(invalid("Git hash collision"));
            }
        } else {
            self.budget.add(raw.len())?;
            self.objects.insert(oid, (kind, raw));
        }
        Ok(oid)
    }

    pub fn tree(&mut self, store: &GitStore, engine: &Engine, root: Id) -> Result<GitOid> {
        let result = (|| {
            if self.failed {
                return Err(invalid("Git preflight plan has failed"));
            }
            engine.verify(root, limits())?;
            self.directory(store, engine, root, 0).map(|(oid, _)| oid)
        })();
        self.failed |= result.is_err();
        result
    }

    fn directory(
        &mut self,
        store: &GitStore,
        engine: &Engine,
        root: Id,
        depth: usize,
    ) -> Result<(GitOid, Bounds)> {
        if depth > 256 {
            return Err(Error::Limit("Git export nesting"));
        }
        if let Some(result) = self.roots.get(&root) {
            return Ok(*result);
        }
        let entries = engine.list(root, &[])?;
        let mut output = Vec::with_capacity(entries.len());
        let mut bounds = Bounds::default();
        for (name, entry) in entries {
            crate::validate_name(&name)?;
            let (mode, oid, child) = match entry {
                Entry::Directory(id) => {
                    let (oid, bounds) = self.directory(store, engine, id, depth + 1)?;
                    if bounds.entries == 0 {
                        return Err(invalid("native empty directory entries cannot be exported"));
                    }
                    (Mode::Directory, oid, bounds)
                }
                Entry::File { blob, executable } => (
                    if executable {
                        Mode::Executable
                    } else {
                        Mode::File
                    },
                    self.blob(store, engine, blob)?,
                    Bounds::default(),
                ),
                Entry::Symlink(blob) => (
                    Mode::Symlink,
                    self.blob(store, engine, blob)?,
                    Bounds::default(),
                ),
                _ => {
                    return Err(invalid(
                        "native conflicts/absent bindings cannot be exported",
                    ));
                }
            };
            bounds.entry(&name, child)?;
            output.push((sort_key(&name, mode), name, mode, oid));
        }
        output.sort_by(|a, b| a.0.cmp(&b.0));
        let mut raw = Vec::new();
        for (_, name, mode, oid) in output {
            raw.extend_from_slice(mode.bytes());
            raw.push(b' ');
            raw.extend_from_slice(&name);
            raw.push(0);
            raw.extend_from_slice(&oid.0);
            if raw.len() > MAX_OBJECT_BYTES {
                return Err(Error::Limit("Git tree payload"));
            }
        }
        let oid = self.add(store, Kind::Tree, raw)?;
        self.roots.insert(root, (oid, bounds));
        Ok((oid, bounds))
    }

    fn blob(&mut self, store: &GitStore, engine: &Engine, id: Id) -> Result<GitOid> {
        if let Some(oid) = self.blobs.get(&id) {
            return Ok(*oid);
        }
        let oid = match engine.object(id)? {
            Object::Blob(raw) => {
                if raw.len() > MAX_OBJECT_BYTES {
                    return Err(Error::Limit("native blob payload"));
                }
                self.add(store, Kind::Blob, raw.clone())
            }
            _ => Err(invalid("native file does not reference a blob")),
        }?;
        self.blobs.insert(id, oid);
        Ok(oid)
    }

    pub fn commit(
        &mut self,
        store: &GitStore,
        revision_id: Id,
        revision: &Revision,
        parents: &[GitOid],
    ) -> Result<GitOid> {
        let result = (|| {
            if self.failed {
                return Err(invalid("Git preflight plan has failed"));
            }
            if Object::Revision(revision.clone()).id()? != revision_id {
                return Err(invalid("native revision ID mismatch"));
            }
            validate_identity(&revision.author)?;
            validate_message(&revision.message)?;
            if revision.time_ns < 0 {
                return Err(invalid("negative Git export timestamps are unsupported"));
            }
            if parents.len() != revision.parents.len()
                || parents.len() > 16
                || parents.iter().collect::<BTreeSet<_>>().len() != parents.len()
            {
                return Err(invalid(
                    "Git parents must correspond to ordered native parents",
                ));
            }
            let (tree, _) = self
                .roots
                .get(&revision.root)
                .ok_or_else(|| invalid("revision source tree was not preflighted"))?;
            for parent in parents {
                if let Some((kind, _)) = self.objects.get(parent) {
                    if *kind != Kind::Commit {
                        return Err(invalid("Git parent is not a commit"));
                    }
                } else {
                    store.object(*parent, Kind::Commit, MAX_OBJECT_BYTES)?;
                }
            }
            let mut raw = format!("tree {}\n", tree.hex());
            for parent in parents {
                raw.push_str(&format!("parent {}\n", parent.hex()));
            }
            let minutes = revision.timezone_minutes;
            let zone = format!(
                "{}{:02}{:02}",
                if minutes < 0 { '-' } else { '+' },
                minutes.unsigned_abs() / 60,
                minutes.unsigned_abs() % 60
            );
            let identity = format!(
                "{} {} {}",
                revision.author,
                revision.time_ns.div_euclid(1_000_000_000),
                zone
            );
            raw.push_str(&format!("author {identity}\ncommitter {identity}\navc-revision {revision_id}\navc-time-ns {}\n\n", revision.time_ns));
            raw.push_str(&revision.message);
            self.add(store, Kind::Commit, raw.into_bytes())
        })();
        self.failed |= result.is_err();
        result
    }

    pub fn write(&self, store: &GitStore) -> Result<()> {
        if self.failed {
            return Err(invalid("Git preflight plan has failed"));
        }
        for (oid, (kind, raw)) in &self.objects {
            if store.hash(*kind, raw, true)? != *oid {
                return Err(invalid("Git write hash mismatch"));
            }
            // hash-object -w may succeed without replacing an existing corrupt
            // loose object. Read and independently hash the actual stored bytes.
            let actual = store.object(*oid, *kind, MAX_OBJECT_BYTES)?;
            if actual != *raw {
                return Err(invalid("stored Git candidate differs from plan"));
            }
        }
        Ok(())
    }
}

fn parse_commit(oid: GitOid, raw: Vec<u8>) -> Result<GitCommit> {
    if raw.contains(&0) {
        return Err(invalid("NUL-containing Git commits are unsupported"));
    }
    let split = raw
        .windows(2)
        .position(|bytes| bytes == b"\n\n")
        .ok_or_else(|| invalid("Git commit lacks header separator"))?;
    let mut tree = None;
    let mut parents = Vec::new();
    let mut author = None;
    let mut committer = None;
    let mut continuation = false;
    for (index, line) in raw[..split].split(|b| *b == b'\n').enumerate() {
        if line.starts_with(b" ") {
            if !continuation {
                return Err(invalid("continuation on required Git header"));
            }
            continue;
        }
        let space = line
            .iter()
            .position(|b| *b == b' ')
            .ok_or_else(|| invalid("invalid Git commit header"))?;
        let (key, value) = (&line[..space], &line[space + 1..]);
        if key.is_empty() || !key.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'-') {
            return Err(invalid("invalid Git commit header"));
        }
        continuation = false;
        match key {
            b"tree" => {
                if tree.is_some() || index != 0 {
                    return Err(invalid("Git tree must be the unique first header"));
                }
                tree = Some(header_oid(value)?);
            }
            b"parent" => {
                if tree.is_none() || author.is_some() {
                    return Err(invalid("Git parents must immediately follow tree"));
                }
                if parents.len() >= 16 {
                    return Err(Error::Limit("Git commit parents (maximum 16)"));
                }
                let parent = header_oid(value)?;
                if parents.contains(&parent) {
                    return Err(invalid("duplicate Git parent"));
                }
                parents.push(parent);
            }
            b"author" => {
                if tree.is_none() || author.is_some() || committer.is_some() {
                    return Err(invalid("Git author must uniquely follow tree/parents"));
                }
                author = Some(parse_person(value)?);
            }
            b"committer" => {
                if author.is_none() || committer.is_some() {
                    return Err(invalid("Git committer must uniquely follow author"));
                }
                committer = Some(parse_person(value)?);
            }
            _ => {
                if committer.is_none() {
                    return Err(invalid("extra Git headers must follow committer"));
                }
                continuation = true;
            }
        }
    }
    let tree = tree.ok_or_else(|| invalid("missing Git tree header"))?;
    let (author, _, _) = author.ok_or_else(|| invalid("missing Git author"))?;
    let author = std::str::from_utf8(author).map_err(|_| invalid("Git author is not UTF-8"))?;
    validate_identity(author)?;
    let author = author.to_owned();
    let (_, seconds, timezone_minutes) =
        committer.ok_or_else(|| invalid("missing Git committer"))?;
    let time_ns = seconds
        .checked_mul(1_000_000_000)
        .ok_or_else(|| invalid("Git committer timestamp outside native range"))?;
    let message =
        std::str::from_utf8(&raw[split + 2..]).map_err(|_| invalid("Git message is not UTF-8"))?;
    validate_message(message)?;
    let message = message.to_owned();
    Ok(GitCommit {
        oid,
        tree,
        parents,
        raw,
        author,
        message,
        time_ns,
        timezone_minutes,
    })
}
fn header_oid(raw: &[u8]) -> Result<GitOid> {
    if raw.len() != 40 {
        return Err(invalid("Git header requires a bare SHA-1 OID"));
    }
    std::str::from_utf8(raw)
        .map_err(|_| invalid("invalid Git header OID"))?
        .parse()
}
fn validate_message(message: &str) -> Result<()> {
    if message.len() > MAX_MESSAGE {
        return Err(Error::Limit("Git message"));
    }
    if message.contains('\0') {
        return Err(invalid(
            "NUL-containing Git commit messages are unsupported",
        ));
    }
    Ok(())
}
fn validate_identity(identity: &str) -> Result<()> {
    if identity.len() > MAX_AUTHOR || identity.bytes().any(|b| matches!(b, 0 | b'\r' | b'\n')) {
        return Err(invalid(
            "Git identity must be bounded and single-line without NUL",
        ));
    }
    validate_raw_identity(identity.as_bytes())
}
fn validate_raw_identity(identity: &[u8]) -> Result<()> {
    let split = identity
        .windows(2)
        .rposition(|b| b == b" <")
        .ok_or_else(|| invalid("Git identity requires Name <email>"))?;
    let name = &identity[..split];
    let email = identity[split + 2..]
        .strip_suffix(b">")
        .ok_or_else(|| invalid("Git identity requires Name <email>"))?;
    if name.is_empty()
        || email.is_empty()
        || name
            .iter()
            .chain(email)
            .any(|b| matches!(b, b'<' | b'>' | 0 | b'\r' | b'\n'))
    {
        return Err(invalid("Git identity requires Name <email>"));
    }
    Ok(())
}
fn parse_person(raw: &[u8]) -> Result<(&[u8], i64, i16)> {
    let mut fields = raw.rsplitn(3, |b| *b == b' ');
    let zone = fields
        .next()
        .ok_or_else(|| invalid("missing Git timezone"))?;
    let seconds = fields
        .next()
        .ok_or_else(|| invalid("missing Git timestamp"))?;
    let identity = fields
        .next()
        .ok_or_else(|| invalid("missing Git identity"))?;
    validate_raw_identity(identity)?;
    let seconds = std::str::from_utf8(seconds).map_err(|_| invalid("invalid Git timestamp"))?;
    if seconds.is_empty()
        || !seconds.bytes().all(|b| b.is_ascii_digit())
        || (seconds.len() > 1 && seconds.starts_with('0'))
    {
        return Err(invalid("invalid Git timestamp"));
    }
    let seconds = seconds
        .parse()
        .map_err(|_| invalid("Git timestamp out of range"))?;
    if zone.len() != 5
        || !matches!(zone[0], b'+' | b'-')
        || !zone[1..].iter().all(u8::is_ascii_digit)
    {
        return Err(invalid("invalid Git timezone"));
    }
    let hour = i16::from(zone[1] - b'0') * 10 + i16::from(zone[2] - b'0');
    let minute = i16::from(zone[3] - b'0') * 10 + i16::from(zone[4] - b'0');
    if hour > 23 || minute > 59 {
        return Err(invalid("Git timezone outside +/-2359"));
    }
    let minutes = (hour * 60 + minute) * if zone[0] == b'-' { -1 } else { 1 };
    Ok((identity, seconds, minutes))
}
fn limits() -> Limits {
    Limits {
        objects: MAX_OBJECTS,
        bindings: MAX_ENTRIES,
        bytes: MAX_CLOSURE_BYTES,
    }
}
fn invalid(message: &str) -> Error {
    Error::Invalid(message.to_owned())
}
