//! Independent typed interpretation of design.md §§3 and 12, not an engine.
use std::collections::BTreeSet;

pub type Result<T> = std::result::Result<T, String>;
pub type Path = Vec<Vec<u8>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Id {
    pub kind: u8,
    pub digest: [u8; 32],
}

impl Id {
    pub fn hash(kind: u8, payload: &[u8]) -> Self {
        let mut hash = blake3::Hasher::new();
        hash.update(b"AVC-OBJECT\0v1\0");
        hash.update(&[kind]);
        hash.update(&(payload.len() as u64).to_le_bytes());
        hash.update(payload);
        Self {
            kind,
            digest: *hash.finalize().as_bytes(),
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != 33 || bytes[0] == 0 {
            return Err("invalid typed ID".into());
        }
        let mut digest = [0; 32];
        digest.copy_from_slice(&bytes[1..]);
        Ok(Self {
            kind: bytes[0],
            digest,
        })
    }

    pub fn display(self) -> String {
        const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
        let mut text = format!("avc1-b3-k{:02x}-", self.kind);
        for group in 0..52 {
            let mut value = 0;
            for bit in 0..5 {
                let offset = group * 5 + bit;
                value <<= 1;
                if offset < 256 {
                    value |= (self.digest[offset / 8] >> (7 - offset % 8)) & 1;
                }
            }
            text.push(char::from(ALPHABET[usize::from(value)]));
        }
        text
    }

    pub fn parse(text: &str) -> Result<Self> {
        let bytes = text.as_bytes();
        if bytes.len() != 64 || &bytes[..9] != b"avc1-b3-k" || bytes[11] != b'-' {
            return Err("invalid external ID".into());
        }
        let kind = unhex(&bytes[9..11])?[0];
        if kind == 0 {
            return Err("zero ID kind".into());
        }
        let mut digest = [0; 32];
        for (group, ch) in bytes[12..].iter().enumerate() {
            let value = match ch {
                b'a'..=b'z' => ch - b'a',
                b'2'..=b'7' => ch - b'2' + 26,
                _ => return Err("base32 alphabet".into()),
            };
            for bit in 0..5 {
                let offset = group * 5 + bit;
                let set = (value >> (4 - bit)) & 1;
                if offset < 256 {
                    digest[offset / 8] |= set << (7 - offset % 8);
                } else if set != 0 {
                    return Err("base32 padding".into());
                }
            }
        }
        Ok(Self { kind, digest })
    }
}

pub fn unhex(bytes: &[u8]) -> Result<Vec<u8>> {
    fn digit(value: u8) -> Result<u8> {
        match value {
            b'0'..=b'9' => Ok(value - b'0'),
            b'a'..=b'f' => Ok(value - b'a' + 10),
            _ => Err("noncanonical hex".into()),
        }
    }
    if !bytes.len().is_multiple_of(2) {
        return Err("odd hex length".into());
    }
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| Ok((digit(pair[0])? << 4) | digit(pair[1])?))
        .collect()
}

pub fn name_valid(name: &[u8]) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name != b"."
        && name != b".."
        && !name.contains(&0)
        && !name.contains(&b'/')
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Entry {
    File(Id, bool),
    Symlink(Id),
    Directory(Id),
    Conflict(Id),
}
impl Entry {
    pub fn id(&self) -> Id {
        match self {
            Self::File(id, _) | Self::Symlink(id) | Self::Directory(id) | Self::Conflict(id) => *id,
        }
    }
}

#[derive(Debug)]
pub enum Structural {
    Move(Path, Path, Entry, Option<Entry>),
    Copy(Entry, Path, Option<Entry>),
}
#[derive(Debug)]
pub struct Edit {
    pub path: Path,
    pub expected: Option<Entry>,
    pub value: Option<Entry>,
}
#[derive(Debug)]
pub enum MergeInputs {
    History(u32, u32, bool),
    Source([Id; 3]),
}
#[derive(Debug)]
pub struct Resolution {
    pub path: Path,
    pub conflict: Id,
    pub value: Option<Entry>,
}
#[derive(Debug)]
pub enum Record {
    Transform {
        base: Id,
        structural: Vec<Structural>,
        edits: Vec<Edit>,
    },
    Merge {
        inputs: MergeInputs,
        algorithms: [(u16, u32); 2],
        moves: [Option<Id>; 2],
    },
    Observation {
        base: Id,
        revision: Option<Id>,
        generation: u64,
        consistency: u8,
        receipt: Option<[u8; 16]>,
        partial: bool,
        coverage_mode: u8,
        scope: Vec<Path>,
        coverage: [Vec<Path>; 3],
        resolutions: Vec<Resolution>,
    },
}
impl Record {
    pub fn tag(&self) -> u8 {
        match self {
            Self::Transform { .. } => 1,
            Self::Merge { .. } => 2,
            Self::Observation { .. } => 3,
        }
    }
}
#[derive(Debug)]
pub struct Occurrence {
    pub path: Path,
    pub conflict: Id,
    pub roles: Vec<u32>,
}
#[derive(Debug)]
pub enum Object {
    Blob(Vec<u8>),
    Leaf(Vec<(Vec<u8>, Entry)>), // nibble suffixes, not full names
    Branch {
        prefix: Vec<u8>,
        terminal: Option<Entry>,
        children: Vec<(u8, Id)>,
    },
    State(Entry),
    Revision {
        state: Id,
        parents: Vec<Id>,
        change: [u8; 16],
        author: Vec<u8>,
        description: Vec<u8>,
        time: i64,
        provenance: Option<Id>,
    },
    Conflict {
        region: u8,
        base: Option<Entry>,
        alternatives: [Option<Entry>; 2],
    },
    Provenance {
        result: Id,
        inputs: Vec<Id>,
        occurrences: Vec<Occurrence>,
        records: Vec<Record>,
    },
}

// Encoder writes typed fields independently of the cursor; no saved payload
// slices or opaque record bodies are used for round-trip qualification.
#[derive(Default)]
pub struct Writer(pub Vec<u8>);
impl Writer {
    pub fn u8(&mut self, n: u8) {
        self.0.push(n);
    }
    pub fn u16(&mut self, n: u16) {
        self.0.extend(n.to_le_bytes());
    }
    pub fn u32(&mut self, n: u32) {
        self.0.extend(n.to_le_bytes());
    }
    pub fn u64(&mut self, n: u64) {
        self.0.extend(n.to_le_bytes());
    }
    pub fn count(&mut self, n: usize) -> Result<()> {
        self.u32(n.try_into().map_err(|_| "array length overflow")?);
        Ok(())
    }
    pub fn nibbles(&mut self, ns: &[u8]) -> Result<()> {
        self.u16(ns.len().try_into().map_err(|_| "nibble length overflow")?);
        for pair in ns.chunks(2) {
            self.u8((pair[0] << 4) | pair.get(1).copied().unwrap_or(0));
        }
        Ok(())
    }
}
pub trait Encode {
    fn write(&self, out: &mut Writer) -> Result<()>;
    fn encoded(&self) -> Result<Vec<u8>> {
        let mut out = Writer::default();
        self.write(&mut out)?;
        Ok(out.0)
    }
}
impl Encode for u8 {
    fn write(&self, out: &mut Writer) -> Result<()> {
        out.u8(*self);
        Ok(())
    }
}
impl Encode for u32 {
    fn write(&self, out: &mut Writer) -> Result<()> {
        out.u32(*self);
        Ok(())
    }
}
impl Encode for bool {
    fn write(&self, out: &mut Writer) -> Result<()> {
        out.u8(u8::from(*self));
        Ok(())
    }
}
impl<T: Encode> Encode for Vec<T> {
    fn write(&self, out: &mut Writer) -> Result<()> {
        out.count(self.len())?;
        for value in self {
            value.write(out)?;
        }
        Ok(())
    }
}
impl<T: Encode> Encode for Option<T> {
    fn write(&self, out: &mut Writer) -> Result<()> {
        self.is_some().write(out)?;
        if let Some(value) = self {
            value.write(out)?;
        }
        Ok(())
    }
}
impl Encode for [u8; 16] {
    fn write(&self, out: &mut Writer) -> Result<()> {
        out.0.extend(self);
        Ok(())
    }
}
impl Encode for Id {
    fn write(&self, out: &mut Writer) -> Result<()> {
        out.u8(self.kind);
        out.0.extend(self.digest);
        Ok(())
    }
}
impl Encode for Entry {
    fn write(&self, out: &mut Writer) -> Result<()> {
        out.u8(match self {
            Self::File(..) => 1,
            Self::Symlink(_) => 2,
            Self::Directory(_) => 3,
            Self::Conflict(_) => 4,
        });
        self.id().write(out)?;
        if let Self::File(_, mode) = self {
            mode.write(out)?;
        }
        Ok(())
    }
}
impl Encode for Structural {
    fn write(&self, out: &mut Writer) -> Result<()> {
        match self {
            Self::Move(source, destination, expected, previous) => {
                out.u8(1);
                source.write(out)?;
                destination.write(out)?;
                expected.write(out)?;
                previous.write(out)?;
            }
            Self::Copy(value, destination, previous) => {
                out.u8(2);
                value.write(out)?;
                destination.write(out)?;
                previous.write(out)?;
            }
        }
        Ok(())
    }
}
impl Encode for Edit {
    fn write(&self, out: &mut Writer) -> Result<()> {
        self.path.write(out)?;
        self.expected.write(out)?;
        self.value.write(out)
    }
}
impl Encode for Resolution {
    fn write(&self, out: &mut Writer) -> Result<()> {
        self.path.write(out)?;
        self.conflict.write(out)?;
        self.value.write(out)
    }
}
impl Encode for Occurrence {
    fn write(&self, out: &mut Writer) -> Result<()> {
        self.path.write(out)?;
        self.conflict.write(out)?;
        self.roles.write(out)
    }
}
impl Encode for Record {
    fn write(&self, out: &mut Writer) -> Result<()> {
        out.u8(self.tag());
        match self {
            Self::Transform {
                base,
                structural,
                edits,
            } => {
                base.write(out)?;
                structural.write(out)?;
                edits.write(out)?;
            }
            Self::Merge {
                inputs,
                algorithms,
                moves,
            } => {
                match inputs {
                    MergeInputs::History(left, right, permit) => {
                        out.u8(0);
                        left.write(out)?;
                        right.write(out)?;
                        permit.write(out)?;
                    }
                    MergeInputs::Source(states) => {
                        out.u8(1);
                        for state in states {
                            state.write(out)?;
                        }
                    }
                }
                for (algorithm, version) in algorithms {
                    out.u16(*algorithm);
                    out.u32(*version);
                }
                for value in moves {
                    value.write(out)?;
                }
            }
            Self::Observation {
                base,
                revision,
                generation,
                consistency,
                receipt,
                partial,
                coverage_mode,
                scope,
                coverage,
                resolutions,
            } => {
                base.write(out)?;
                revision.write(out)?;
                out.u64(*generation);
                out.u8(*consistency);
                receipt.write(out)?;
                partial.write(out)?;
                out.u8(*coverage_mode);
                scope.write(out)?;
                for paths in coverage {
                    paths.write(out)?;
                }
                resolutions.write(out)?;
            }
        }
        Ok(())
    }
}
impl Object {
    pub fn kind(&self) -> u8 {
        match self {
            Self::Blob(_) => 1,
            Self::Leaf(_) => 2,
            Self::Branch { .. } => 3,
            Self::State(_) => 4,
            Self::Revision { .. } => 5,
            Self::Conflict { .. } => 6,
            Self::Provenance { .. } => 7,
        }
    }
}
impl Encode for Object {
    fn write(&self, out: &mut Writer) -> Result<()> {
        match self {
            Self::Blob(data) => out.0.extend(data),
            Self::Leaf(items) => {
                out.count(items.len())?;
                for (tail, value) in items {
                    out.nibbles(tail)?;
                    value.write(out)?;
                }
            }
            Self::Branch {
                prefix,
                terminal,
                children,
            } => {
                out.nibbles(prefix)?;
                terminal.write(out)?;
                out.u16(
                    children
                        .iter()
                        .fold(0, |mask, (slot, _)| mask | (1 << slot)),
                );
                for (_, child) in children {
                    child.write(out)?;
                }
            }
            Self::State(entry) => entry.write(out)?,
            Self::Revision {
                state,
                parents,
                change,
                author,
                description,
                time,
                provenance,
            } => {
                state.write(out)?;
                parents.write(out)?;
                change.write(out)?;
                author.write(out)?;
                description.write(out)?;
                out.0.extend(time.to_le_bytes());
                provenance.write(out)?;
            }
            Self::Conflict {
                region,
                base,
                alternatives,
            } => {
                out.u8(*region);
                base.write(out)?;
                for entry in alternatives {
                    entry.write(out)?;
                }
            }
            Self::Provenance {
                result,
                inputs,
                occurrences,
                records,
            } => {
                result.write(out)?;
                out.u8(1);
                inputs.write(out)?;
                occurrences.write(out)?;
                out.u8(records
                    .len()
                    .try_into()
                    .map_err(|_| "record count overflow")?);
                for record in records {
                    record.write(out)?;
                }
            }
        }
        Ok(())
    }
}

struct Reader<'a> {
    data: &'a [u8],
    position: usize,
    audit: BTreeSet<Id>,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if n > self.data.len() - self.position {
            return Err("truncated field".into());
        }
        let data = &self.data[self.position..self.position + n];
        self.position += n;
        Ok(data)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.take(N)?
            .try_into()
            .map_err(|_| "fixed field length".into())
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }
    fn flag(&mut self) -> Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err("nonbinary flag".into()),
        }
    }
    fn option<T>(&mut self, parse: impl FnOnce(&mut Self) -> Result<T>) -> Result<Option<T>> {
        if self.flag()? {
            Ok(Some(parse(self)?))
        } else {
            Ok(None)
        }
    }
    fn count(&mut self, limit: usize, minimum: usize) -> Result<usize> {
        let n = usize::try_from(self.u32()?).map_err(|_| "count overflow")?;
        if n > limit || n > (self.data.len() - self.position) / minimum {
            return Err("array count/budget".into());
        }
        Ok(n)
    }
    fn list<T>(
        &mut self,
        limit: usize,
        minimum: usize,
        mut parse: impl FnMut(&mut Self) -> Result<T>,
    ) -> Result<Vec<T>> {
        let count = self.count(limit, minimum)?;
        let mut values = Vec::new();
        for _ in 0..count {
            values.push(parse(self)?);
        }
        Ok(values)
    }
    fn bytes(&mut self, limit: usize) -> Result<Vec<u8>> {
        let n = self.count(limit, 1)?;
        Ok(self.take(n)?.to_vec())
    }
    fn id(&mut self, kinds: &[u8], audit: bool) -> Result<Id> {
        let id = Id::from_bytes(self.take(33)?)?;
        if !kinds.contains(&id.kind) {
            return Err("wrong reference kind".into());
        }
        if audit {
            self.audit.insert(id);
        }
        Ok(id)
    }
    fn entry(&mut self, audit: bool) -> Result<Entry> {
        match self.u8()? {
            1 => Ok(Entry::File(self.id(&[1], audit)?, self.flag()?)),
            2 => Ok(Entry::Symlink(self.id(&[1], audit)?)),
            3 => Ok(Entry::Directory(self.id(&[2, 3], audit)?)),
            4 => Ok(Entry::Conflict(self.id(&[6], audit)?)),
            _ => Err("entry tag".into()),
        }
    }
    fn maybe(&mut self, audit: bool) -> Result<Option<Entry>> {
        self.option(|c| c.entry(audit))
    }
    fn nibbles(&mut self) -> Result<Vec<u8>> {
        let n = usize::from(self.u16()?);
        if n > 510 {
            return Err("nibble name budget".into());
        }
        let raw = self.take(n.div_ceil(2))?;
        if !n.is_multiple_of(2) && raw.last().is_some_and(|b| b & 15 != 0) {
            return Err("nibble padding".into());
        }
        Ok((0..n)
            .map(|i| {
                if i % 2 == 0 {
                    raw[i / 2] >> 4
                } else {
                    raw[i / 2] & 15
                }
            })
            .collect())
    }
    fn path(&mut self, nonempty: bool) -> Result<Path> {
        let parts = self.list(256, 4, |c| c.bytes(255))?;
        if (nonempty && parts.is_empty())
            || parts.iter().any(|p| !name_valid(p))
            || parts.iter().map(Vec::len).sum::<usize>() + parts.len().saturating_sub(1) > 4096
        {
            return Err("path constraints".into());
        }
        Ok(parts)
    }
    fn paths(&mut self) -> Result<Vec<Path>> {
        let paths = self.list(1_000_000, 4, |c| c.path(false))?;
        if paths.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err("path ordering".into());
        }
        Ok(paths)
    }
    fn record(&mut self, input_count: usize) -> Result<Record> {
        match self.u8()? {
            1 => {
                let base = self.id(&[4], true)?;
                let structural = self.list(1_000_000, 1, |c| match c.u8()? {
                    1 => Ok(Structural::Move(
                        c.path(true)?,
                        c.path(true)?,
                        c.entry(true)?,
                        c.maybe(true)?,
                    )),
                    2 => Ok(Structural::Copy(
                        c.entry(true)?,
                        c.path(true)?,
                        c.maybe(true)?,
                    )),
                    _ => Err("structural tag".into()),
                })?;
                let edits = self.list(1_000_000, 6, |c| {
                    Ok(Edit {
                        path: c.path(true)?,
                        expected: c.maybe(true)?,
                        value: c.maybe(true)?,
                    })
                })?;
                Ok(Record::Transform {
                    base,
                    structural,
                    edits,
                })
            }
            2 => {
                let inputs = match self.u8()? {
                    0 => {
                        let left = self.u32()?;
                        let right = self.u32()?;
                        if u64::from(left) >= input_count as u64
                            || u64::from(right) >= input_count as u64
                        {
                            return Err("history input index".into());
                        }
                        MergeInputs::History(left, right, self.flag()?)
                    }
                    1 => MergeInputs::Source([
                        self.id(&[4], true)?,
                        self.id(&[4], true)?,
                        self.id(&[4], true)?,
                    ]),
                    _ => return Err("merge mode".into()),
                };
                let mut algorithms = [(0, 0); 2];
                for (index, pair) in algorithms.iter_mut().enumerate() {
                    *pair = (self.u16()?, self.u32()?);
                    if (pair.0 == 0 && (index == 0 || pair.1 != 0)) || (pair.0 != 0 && pair.1 == 0)
                    {
                        return Err("algorithm version".into());
                    }
                }
                let moves = [
                    self.option(|c| c.id(&[7], true))?,
                    self.option(|c| c.id(&[7], true))?,
                ];
                Ok(Record::Merge {
                    inputs,
                    algorithms,
                    moves,
                })
            }
            3 => {
                let base = self.id(&[4], true)?;
                let revision = self.option(|c| c.id(&[5], true))?;
                let generation = self.u64()?;
                let consistency = self.u8()?;
                if consistency > 2 {
                    return Err("consistency label".into());
                }
                let receipt = self.option(Self::array)?;
                let partial = self.flag()?;
                let coverage_mode = self.u8()?;
                if coverage_mode > 1 {
                    return Err("coverage mode".into());
                }
                let scope = self.paths()?;
                if scope.is_empty() || scope.windows(2).any(|p| p[1].starts_with(&p[0])) {
                    return Err("observation scope".into());
                }
                let coverage = [self.paths()?, self.paths()?, self.paths()?];
                let mut seen = BTreeSet::new();
                for paths in &coverage {
                    for path in paths {
                        if !within(&scope, path) || !seen.insert(path) {
                            return Err("coverage overlap/scope".into());
                        }
                    }
                }
                if coverage_mode == 1 && !seen.is_empty() {
                    return Err("completed scope has entry lists".into());
                }
                if !partial && (!coverage[1].is_empty() || !coverage[2].is_empty()) {
                    return Err("unconfirmed partial".into());
                }
                let resolutions = self.list(1_000_000, 38, |c| {
                    Ok(Resolution {
                        path: c.path(false)?,
                        conflict: c.id(&[6], true)?,
                        value: c.maybe(true)?,
                    })
                })?;
                if resolutions.windows(2).any(|p| p[0].path >= p[1].path)
                    || resolutions.iter().any(|r| !within(&scope, &r.path))
                {
                    return Err("resolution order/scope".into());
                }
                Ok(Record::Observation {
                    base,
                    revision,
                    generation,
                    consistency,
                    receipt,
                    partial,
                    coverage_mode,
                    scope,
                    coverage,
                    resolutions,
                })
            }
            _ => Err("unknown record".into()),
        }
    }
}
fn within(scope: &[Path], path: &Path) -> bool {
    // Binary search the last prefix no greater than the path, rather than scan
    // every scope for every observation (wide scopes are legitimate).
    let end = scope.partition_point(|prefix| prefix <= path);
    end > 0 && path.starts_with(&scope[end - 1])
}

pub fn decode(kind: u8, payload: &[u8]) -> Result<(Object, BTreeSet<Id>)> {
    if kind == 7 && payload.len() > 1_048_576 {
        return Err("provenance size".into());
    }
    let mut c = Reader {
        data: payload,
        position: 0,
        audit: BTreeSet::new(),
    };
    let object = match kind {
        1 => {
            c.position = payload.len();
            Object::Blob(payload.to_vec())
        }
        2 => {
            if payload.len() > 4096 {
                return Err("leaf size".into());
            }
            Object::Leaf(c.list(256, 36, |c| Ok((c.nibbles()?, c.entry(false)?)))?)
        }
        3 => {
            let prefix = c.nibbles()?;
            let terminal = c.maybe(false)?;
            let bitmap = c.u16()?;
            if bitmap == 0 || bitmap.count_ones() + u32::from(terminal.is_some()) < 2 {
                return Err("uncompressed branch".into());
            }
            let mut children = Vec::new();
            for slot in 0..16 {
                if bitmap & (1 << slot) != 0 {
                    children.push((slot, c.id(&[2, 3], false)?));
                }
            }
            Object::Branch {
                prefix,
                terminal,
                children,
            }
        }
        4 => Object::State(c.entry(false)?),
        5 => {
            let state = c.id(&[4], false)?;
            let parents = c.list(64, 33, |c| c.id(&[5], false))?;
            if parents.iter().collect::<BTreeSet<_>>().len() != parents.len() {
                return Err("duplicate parents".into());
            }
            Object::Revision {
                state,
                parents,
                change: c.array()?,
                author: c.bytes(4096)?,
                description: c.bytes(1_048_576)?,
                time: i64::from_le_bytes(c.array()?),
                provenance: c.option(|c| c.id(&[7], false))?,
            }
        }
        6 => {
            let region = c.u8()?;
            if region > 1 {
                return Err("conflict region".into());
            }
            let base = c.maybe(false)?;
            let alternatives = [c.maybe(false)?, c.maybe(false)?];
            if alternatives[0].encoded()? >= alternatives[1].encoded()? {
                return Err("conflict alternative ordering".into());
            }
            Object::Conflict {
                region,
                base,
                alternatives,
            }
        }
        7 => {
            let result = c.id(&[4], false)?;
            if c.u8()? != 1 {
                return Err("provenance binding".into());
            }
            let inputs = c.list(64, 33, |c| c.id(&[5], true))?;
            if inputs.iter().collect::<BTreeSet<_>>().len() != inputs.len() {
                return Err("duplicate inputs".into());
            }
            let occurrences = c.list(1_000_000, 41, |c| {
                Ok(Occurrence {
                    path: c.path(false)?,
                    conflict: c.id(&[6], true)?,
                    roles: c.list(1_000_000, 4, |c| c.u32())?,
                })
            })?;
            if occurrences
                .windows(2)
                .any(|p| (&p[0].path, p[0].conflict) >= (&p[1].path, p[1].conflict))
                || occurrences
                    .iter()
                    .flat_map(|o| &o.roles)
                    .any(|role| u64::from(*role) >= inputs.len() as u64)
            {
                return Err("occurrence order/role".into());
            }
            let n = c.u8()?;
            if !(1..=2).contains(&n) {
                return Err("record count".into());
            }
            let mut records = Vec::new();
            for _ in 0..n {
                records.push(c.record(inputs.len())?);
            }
            if records
                .windows(2)
                .any(|p| p[0].tag() >= p[1].tag() || (p[0].tag() == 1 && p[1].tag() == 2))
            {
                return Err("record ordering/combination".into());
            }
            Object::Provenance {
                result,
                inputs,
                occurrences,
                records,
            }
        }
        _ => return Err("unknown/retired kind".into()),
    };
    if c.position != payload.len() {
        return Err("trailing data".into());
    }
    Ok((object, c.audit))
}
