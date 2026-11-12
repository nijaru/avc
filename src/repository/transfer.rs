//! Explicit source-only and selected-history transfers. Both use canonical
//! objects, bounded exact owning closures and checked completion, not DB copies.
use super::*;

#[derive(Clone, Copy)]
enum Format {
    Source,
    History,
}
impl Format {
    fn markers(self) -> (&'static [u8; 8], &'static [u8; 8]) {
        match self {
            Self::Source => (b"AVCEXP01", b"AVCEND01"),
            Self::History => (b"AVCHIS01", b"AVCHEND1"),
        }
    }
}

struct Bundle {
    root: Id,
    revision: Option<Id>,
    records: BTreeMap<Id, Vec<u8>>,
}
impl Bundle {
    fn checked(root: Id, revision: Option<Id>, records: BTreeMap<Id, Vec<u8>>) -> Result<Self> {
        check_roots(root, revision)?;
        let count = records.len();
        let mut engine = Engine::new();
        engine.import(records, root, limits())?;
        if let Some(head) = revision {
            engine.verify(head, limits())?;
        }
        let exact = collect_closure(std::iter::once(root).chain(revision), |id| {
            encode_bounded(engine.object(id)?)
        })?;
        if exact.len() != count {
            return Err(invalid("export contains non-owning objects"));
        }
        Ok(Self {
            root,
            revision,
            records: exact,
        })
    }

    fn read(mut reader: impl Read, format: Format) -> Result<Self> {
        let (magic, end) = format.markers();
        let mut hash = blake3::Hasher::new();
        if read_hashed::<8>(&mut reader, &mut hash)? != *magic {
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
        // Reject invalid manifest kinds before reading/allocating payloads.
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
            let length = u64::from_le_bytes(read_hashed::<8>(&mut reader, &mut hash)?);
            let length = bounded_length(length)?;
            budget(&mut total, length)?;
            let mut payload = vec![0; length];
            reader.read_exact(&mut payload)?;
            hash.update(&payload);
            Object::decode(id, &payload)?;
            records.insert(id, payload);
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
        Self::checked(root, revision, records)
    }

    fn write(self, mut writer: impl Write, format: Format) -> Result<()> {
        let (magic, end) = format.markers();
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
        write_hashed(&mut writer, &mut hash, end)?;
        writer.write_all(hash.finalize().as_bytes())?;
        Ok(())
    }
}

impl Repository {
    fn prepare_export(&self, root: Id, revision: Option<Id>) -> Result<Bundle> {
        self.ensure_idle()?;
        check_roots(root, revision)?;
        let records = collect_closure(std::iter::once(root).chain(revision), |id| {
            self.read_object(id)
        })?;
        Bundle::checked(root, revision, records)
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
    /// Caller owns flush/sync/publication; receipts/instance/workspace/Git metadata
    /// are not included. This is selected native history, not all repository state.
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
