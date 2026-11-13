//! Explicit source-only and selected-history transfers. Canonical objects and
//! optional exact Git origins use bounded owning closures and checked completion.
use super::*;
use crate::git::{GitCommit, GitOid, GitTreeHasher};

type RawOrigin = (Id, GitOid, Vec<u8>);

#[derive(Clone, Copy)]
enum Format {
    Source,
    History,
}
impl Format {
    fn markers(self, origins: bool) -> (&'static [u8; 8], &'static [u8; 8]) {
        match self {
            Self::Source => (b"AVCEXP01", b"AVCEND01"),
            Self::History if origins => (b"AVCHIS02", b"AVCHEND2"),
            Self::History => (b"AVCHIS01", b"AVCHEND1"),
        }
    }
}

struct Bundle {
    root: Id,
    revision: Option<Id>,
    records: BTreeMap<Id, Vec<u8>>,
    origins: Vec<GitOrigin>,
    scope: OriginScope,
}
impl Bundle {
    fn checked(
        root: Id,
        revision: Option<Id>,
        records: BTreeMap<Id, Vec<u8>>,
        raw_origins: Vec<RawOrigin>,
    ) -> Result<Self> {
        check_roots(root, revision)?;
        let count = records.len();
        let mut engine = Engine::new();
        engine.import_roots(records, std::iter::once(root).chain(revision), limits())?;
        let exact = collect_closure(std::iter::once(root).chain(revision), |id| {
            encode_bounded(engine.object(id)?)
        })?;
        if exact.len() != count {
            return Err(invalid("export contains non-owning objects"));
        }
        let origins = check_origins(&engine, revision, &exact, raw_origins)?;
        let scope = OriginScope::new(&origins)?;
        Ok(Self {
            root,
            revision,
            records: exact,
            origins,
            scope,
        })
    }

    fn read(mut reader: impl Read, format: Format) -> Result<Self> {
        let mut hash = blake3::Hasher::new();
        let magic = read_hashed::<8>(&mut reader, &mut hash)?;
        let has_origins = matches!(format, Format::History) && magic == *b"AVCHIS02";
        let (expected, end) = format.markers(has_origins);
        if magic != *expected {
            return Err(invalid("export version/magic"));
        }
        let root = Id::from_bytes(&read_hashed::<33>(&mut reader, &mut hash)?)?;
        let revision = if matches!(format, Format::History) {
            match read_hashed::<1>(&mut reader, &mut hash)?[0] {
                0 => None,
                1 => Some(Id::from_bytes(&read_hashed::<33>(&mut reader, &mut hash)?)?),
                _ => return Err(invalid("history authored-head tag")),
            }
        } else {
            None
        };
        check_roots(root, revision)?;
        let count = u64::from_le_bytes(read_hashed::<8>(&mut reader, &mut hash)?);
        if count == 0 || count > MAX_OBJECTS as u64 {
            return Err(Error::Limit("export object count"));
        }
        let mut records = BTreeMap::new();
        let mut previous = None;
        let mut total = 0;
        for _ in 0..count {
            let id = Id::from_bytes(&read_hashed::<33>(&mut reader, &mut hash)?)?;
            if previous.is_some_and(|p| p >= id) {
                return Err(invalid("export records must be unique and sorted"));
            }
            previous = Some(id);
            let length = bounded_length(u64::from_le_bytes(read_hashed::<8>(
                &mut reader,
                &mut hash,
            )?))?;
            budget(&mut total, length)?;
            let mut payload = vec![0; length];
            reader.read_exact(&mut payload)?;
            hash.update(&payload);
            Object::decode(id, &payload)?;
            records.insert(id, payload);
        }
        let mut origins = Vec::new();
        if has_origins {
            if revision.is_none() {
                return Err(invalid("origin history requires authored head"));
            }
            let count = u64::from_le_bytes(read_hashed::<8>(&mut reader, &mut hash)?);
            if count == 0 || count > MAX_OBJECTS as u64 {
                return Err(Error::Limit("history origin count"));
            }
            let mut previous = None;
            for _ in 0..count {
                let revision = Id::from_bytes(&read_hashed::<33>(&mut reader, &mut hash)?)?;
                if revision.kind != crate::REVISION_KIND
                    || previous.is_some_and(|id| id >= revision)
                {
                    return Err(invalid("history origins must be sorted unique revisions"));
                }
                previous = Some(revision);
                let commit = GitOid::from_bytes(&read_hashed::<21>(&mut reader, &mut hash)?)?;
                let length = bounded_length(u64::from_le_bytes(read_hashed::<8>(
                    &mut reader,
                    &mut hash,
                )?))?;
                budget(&mut total, length)?;
                let mut raw = vec![0; length];
                reader.read_exact(&mut raw)?;
                hash.update(&raw);
                origins.push((revision, commit, raw));
            }
        }
        if read_hashed::<8>(&mut reader, &mut hash)? != *end {
            return Err(invalid("incomplete export"));
        }
        let mut digest = [0; 32];
        reader.read_exact(&mut digest)?;
        if digest != *hash.finalize().as_bytes() {
            return Err(invalid("export checksum"));
        }
        require_eof(&mut reader)?;
        Self::checked(root, revision, records, origins)
    }

    fn write(self, mut writer: impl Write, format: Format) -> Result<()> {
        let (magic, end) = format.markers(!self.origins.is_empty());
        let mut hash = blake3::Hasher::new();
        write_hashed(&mut writer, &mut hash, magic)?;
        write_hashed(&mut writer, &mut hash, &self.root.to_bytes()?)?;
        if matches!(format, Format::History) {
            write_hashed(&mut writer, &mut hash, &[u8::from(self.revision.is_some())])?;
            if let Some(head) = self.revision {
                write_hashed(&mut writer, &mut hash, &head.to_bytes()?)?;
            }
        }
        write_hashed(
            &mut writer,
            &mut hash,
            &(self.records.len() as u64).to_le_bytes(),
        )?;
        for (id, payload) in self.records {
            write_hashed(&mut writer, &mut hash, &id.to_bytes()?)?;
            write_hashed(
                &mut writer,
                &mut hash,
                &(payload.len() as u64).to_le_bytes(),
            )?;
            write_hashed(&mut writer, &mut hash, &payload)?;
        }
        if !self.origins.is_empty() {
            write_hashed(
                &mut writer,
                &mut hash,
                &(self.origins.len() as u64).to_le_bytes(),
            )?;
            for origin in self.origins {
                write_hashed(&mut writer, &mut hash, &origin.revision.to_bytes()?)?;
                write_hashed(&mut writer, &mut hash, &origin.commit.to_bytes())?;
                write_hashed(
                    &mut writer,
                    &mut hash,
                    &(origin.raw_commit.len() as u64).to_le_bytes(),
                )?;
                write_hashed(&mut writer, &mut hash, &origin.raw_commit)?;
            }
        }
        write_hashed(&mut writer, &mut hash, end)?;
        writer.write_all(hash.finalize().as_bytes())?;
        Ok(())
    }
}

fn check_origins(
    engine: &Engine,
    head: Option<Id>,
    records: &BTreeMap<Id, Vec<u8>>,
    raw: Vec<RawOrigin>,
) -> Result<Vec<GitOrigin>> {
    if raw.len() > MAX_OBJECTS {
        return Err(Error::Limit("history origin count"));
    }
    let mut total = 0;
    for payload in records.values() {
        budget(&mut total, payload.len())?;
    }
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    let mut reachable = BTreeSet::new();
    let mut pending = vec![head.ok_or_else(|| invalid("origin history requires authored head"))?];
    while let Some(id) = pending.pop() {
        if reachable.insert(id) {
            let Object::Revision(value) = engine.object(id)? else {
                return Err(invalid("history ancestry requires revisions"));
            };
            pending.extend(&value.parents);
        }
    }
    let mut hasher = GitTreeHasher::new();
    let mut origins = Vec::new();
    let mut previous = None;
    for (revision, commit, raw) in raw {
        bounded_length(raw.len() as u64)?;
        budget(&mut total, raw.len())?;
        if previous.is_some_and(|id| id >= revision) || !reachable.contains(&revision) {
            return Err(invalid("origin is not a sorted unique selected ancestor"));
        }
        previous = Some(revision);
        let Object::Revision(native) = engine.object(revision)? else {
            return Err(invalid("origin revision kind"));
        };
        let original = GitCommit::decode(commit, raw)?;
        if Object::Revision(git_bridge::anchor(&original, native.root)).id()? != revision
            || hasher.tree(engine, native.root)? != original.tree
        {
            return Err(invalid("Git origin does not match original source/anchor"));
        }
        origins.push(GitOrigin {
            revision,
            commit,
            tree: original.tree,
            parents: original.parents,
            raw_commit: original.raw,
        });
    }
    Ok(origins)
}

impl Repository {
    fn prepare_export(&self, root: Id, revision: Option<Id>) -> Result<Bundle> {
        self.ensure_idle()?;
        check_roots(root, revision)?;
        let records = collect_closure(std::iter::once(root).chain(revision), |id| {
            self.read_object(id)
        })?;
        // All queried revisions exist before custody lookup. Decode once, not
        // once per ancestor; immutable rows remain valid if selection advances.
        let scope = if revision.is_some() {
            OriginScope::load(&self.connection)?
        } else {
            None
        };
        let mut total = 0;
        for payload in records.values() {
            budget(&mut total, payload.len())?;
        }
        let mut origins = Vec::new();
        for id in records.keys().filter(|id| id.kind == crate::REVISION_KIND) {
            if let Some(origin) = git_bridge::read_origin(&self.connection, *id, scope.as_ref())? {
                budget(&mut total, origin.raw_commit.len())?;
                origins.push((*id, origin.commit, origin.raw_commit));
            }
        }
        Bundle::checked(root, revision, records, origins)
    }

    /// Export exact source only, preserving the AVCEXP01 framing. No authored
    /// history, local selections, private receipts or orphan objects are included.
    /// Caller owns output flush/sync/publication; failure may leave partial bytes.
    pub fn export(&self, root: Id, writer: impl Write) -> Result<()> {
        self.prepare_export(root, None)?
            .write(writer, Format::Source)
    }

    /// Exclusively create and durably finish a source-only export. Preflight
    /// precedes creation; later I/O failure leaves incomplete output for inspection.
    pub fn export_file(&self, root: Id, path: &Path) -> Result<()> {
        let bundle = self.prepare_export(root, None)?;
        complete_file(path, |output| bundle.write(output, Format::Source))
    }

    /// Export the selected source and optional authored head with their exact
    /// combined owning closure, including ancestor revisions and historical source.
    /// Captures both selections in one SQL read. Immutable rows are never deleted,
    /// so later selection advances cannot invalidate this captured closure.
    /// Returned generation is diagnostic only and is NOT encoded into the stream.
    /// Caller owns flush/sync/publication. Exact Git origins accompany reachable
    /// anchors; receipts, instance, workspace and derived Git maps do not.
    /// This is selected history, not all repository state or foreign Git ancestry.
    pub fn export_history(&self, writer: impl Write) -> Result<Selection> {
        let selected = self.status()?;
        self.prepare_export(selected.root, selected.revision)?
            .write(writer, Format::History)?;
        Ok(selected)
    }

    /// Durably publish a selected-history file by exclusive creation and checked
    /// file/parent syncs. Existing output is never overwritten or automatically removed.
    pub fn export_history_file(&self, path: &Path) -> Result<Selection> {
        let selected = self.status()?;
        let bundle = self.prepare_export(selected.root, selected.revision)?;
        complete_file(path, |output| bundle.write(output, Format::History))?;
        Ok(selected)
    }

    /// Verify a complete source-only stream before admission into an untouched
    /// initialized store. Exact retry returns its original receipt; EOF is required.
    pub fn import(&mut self, reader: impl Read, request_key: &str) -> Result<Receipt> {
        self.import_transfer(reader, request_key, Format::Source)
    }

    /// Restore exact selected source/head, not original generations, request keys
    /// or store identity. All input verification precedes any admission. Payloads,
    /// selected source/head and a new typed receipt publish in one SQLite transaction.
    /// An identical retry may recover the original result after later local edits.
    pub fn import_history(&mut self, reader: impl Read, request_key: &str) -> Result<Receipt> {
        self.import_transfer(reader, request_key, Format::History)
    }

    fn import_transfer(
        &mut self,
        reader: impl Read,
        request_key: &str,
        format: Format,
    ) -> Result<Receipt> {
        self.ensure_idle()?;
        check_request_key(request_key)?;
        let bundle = Bundle::read(reader, format)?;
        let mutation = match format {
            Format::Source => Mutation::Source {
                root: bundle.root,
                empty_only: true,
            },
            Format::History => Mutation::ImportHistory {
                root: bundle.root,
                revision: bundle.revision,
                origins: &bundle.origins,
                scope: &bundle.scope,
            },
        };
        if let Some(prior) = self.receipt(request_key)? {
            return self.reconcile(prior, 0, mutation);
        }
        if self.status()?.generation != 0 {
            if let Some(prior) = self.receipt(request_key)? {
                return self.reconcile(prior, 0, mutation);
            }
            return Err(Error::NotEmpty);
        }
        // Check retained rows before inserting anything: absent dependencies in
        // an old row are corruption, never silently repaired by imported payloads.
        let prepared = self.prepare_objects(bundle.records)?;
        self.publish(0, request_key, prepared, mutation)
    }
}

fn check_roots(root: Id, revision: Option<Id>) -> Result<()> {
    require_source_root(root)?;
    if revision.is_some_and(|id| id.kind != crate::REVISION_KIND) {
        return Err(invalid("history authored head requires a Revision ID"));
    }
    Ok(())
}

fn complete_file<T>(path: &Path, write: impl FnOnce(&mut File) -> Result<T>) -> Result<T> {
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    let result = write(&mut output)?;
    sync_file(&output)?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    sync_directory(parent)?;
    sync_file(&output)?;
    Ok(result)
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
