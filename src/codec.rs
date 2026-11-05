//! Local object validation only: map-root context, referenced objects, cycles,
//! exact subtree counts, and maximal branch prefixes require a closure verifier.

use std::{collections::BTreeMap, fmt, io::Read, str::FromStr};

pub const BLOB_KIND: u8 = 0x01;
pub const MAP_LEAF_KIND: u8 = 0x02;
pub const MAP_BRANCH_KIND: u8 = 0x03;
pub const CONFLICT_KIND: u8 = 0x04;
pub const REVISION_KIND: u8 = 0x05;
const HASH_DOMAIN: &[u8] = b"AVC-OBJECT\0v1\0";
const MAX_NIBBLES: usize = 510;
const MAX_AUTHOR: usize = 4 * 1024;
const MAX_MESSAGE: usize = 1024 * 1024;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Invalid(String),
    NeedObjects(Id),
    Conflict(Id),
    UnresolvedBase(Id),
    Limit(&'static str),
    Stale,
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => write!(f, "invalid AVC object: {reason}"),
            Self::NeedObjects(id) => write!(f, "need object {id}"),
            Self::Conflict(id) => write!(f, "explicit resolution required for conflict {id}"),
            Self::UnresolvedBase(id) => write!(
                f,
                "merge pending: unresolved base {id}; choose an explicit resolved base or replacement"
            ),
            Self::Limit(limit) => write!(f, "resource limit: {limit}"),
            Self::Stale => f.write_str("stale witness"),
            Self::Io(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

fn invalid(reason: &str) -> Error {
    Error::Invalid(reason.to_owned())
}

fn validate_kind(kind: u8) -> Result<()> {
    match kind {
        BLOB_KIND..=REVISION_KIND => Ok(()),
        _ => Err(invalid("unknown object kind")),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Id {
    pub kind: u8,
    pub digest: [u8; 32],
}

impl Id {
    fn hasher(kind: u8, length: u64) -> Result<blake3::Hasher> {
        validate_kind(kind)?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(HASH_DOMAIN);
        hasher.update(&[kind]);
        hasher.update(&length.to_le_bytes());
        Ok(hasher)
    }

    /// Hashes bytes with the typed v1 domain. Does not validate payload grammar.
    pub fn hash(kind: u8, payload: &[u8]) -> Result<Self> {
        let length = u64::try_from(payload.len()).map_err(|_| Error::Limit("payload length"))?;
        let mut hasher = Self::hasher(kind, length)?;
        hasher.update(payload);
        Ok(Self {
            kind,
            digest: *hasher.finalize().as_bytes(),
        })
    }

    /// Hashes an exact-length stream without buffering the payload. Requires EOF
    /// after `length` bytes; use a bounded reader for an embedded payload.
    pub fn hash_reader(kind: u8, length: u64, mut reader: impl Read) -> Result<Self> {
        let mut hasher = Self::hasher(kind, length)?;
        let mut remaining = length;
        let mut buffer = [0; 64 * 1024];
        while remaining != 0 {
            let chunk = usize::try_from(remaining.min(buffer.len() as u64))
                .map_err(|_| Error::Limit("stream chunk length"))?;
            reader.read_exact(&mut buffer[..chunk])?;
            hasher.update(&buffer[..chunk]);
            remaining -= chunk as u64;
        }
        let mut extra = [0];
        loop {
            match reader.read(&mut extra) {
                Ok(0) => break,
                Ok(_) => return Err(invalid("stream exceeds declared payload length")),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Ok(Self {
            kind,
            digest: *hasher.finalize().as_bytes(),
        })
    }

    pub fn to_bytes(self) -> Result<[u8; 33]> {
        validate_kind(self.kind)?;
        let mut bytes = [0; 33];
        bytes[0] = self.kind;
        bytes[1..].copy_from_slice(&self.digest);
        Ok(bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != 33 {
            return Err(invalid("typed reference must have 33 bytes"));
        }
        validate_kind(bytes[0])?;
        let mut digest = [0; 32];
        digest.copy_from_slice(&bytes[1..]);
        Ok(Self {
            kind: bytes[0],
            digest,
        })
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "avc1-k{:02x}-", self.kind)?;
        for byte in self.digest {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

fn hex_digit(byte: u8) -> Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(invalid("ID must use lowercase hexadecimal")),
    }
}

impl FromStr for Id {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        let bytes = text.as_bytes();
        if bytes.len() != 73 || !bytes.starts_with(b"avc1-k") || bytes[8] != b'-' {
            return Err(invalid("noncanonical ID spelling"));
        }
        let kind = (hex_digit(bytes[6])? << 4) | hex_digit(bytes[7])?;
        validate_kind(kind)?;
        let mut digest = [0; 32];
        let (pairs, _) = bytes[9..].as_chunks::<2>();
        for (out, pair) in digest.iter_mut().zip(pairs) {
            *out = (hex_digit(pair[0])? << 4) | hex_digit(pair[1])?;
        }
        Ok(Self { kind, digest })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Entry {
    File { blob: Id, executable: bool },
    Symlink(Id),
    Directory(Id),
    Conflict(Id),
    Absent,
}

// Descriptor ordering is byte ordering, NOT Entry's derived Rust enum ordering.
struct Descriptor {
    bytes: [u8; 35],
    len: usize,
}

impl Descriptor {
    fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

impl Entry {
    fn descriptor(&self) -> Result<Descriptor> {
        let (tag, reference, executable) = match self {
            Self::Absent => (0, None, None),
            Self::File { blob, executable } => {
                require_kind(*blob, &[BLOB_KIND])?;
                (1, Some(*blob), Some(*executable))
            }
            Self::Symlink(blob) => {
                require_kind(*blob, &[BLOB_KIND])?;
                (2, Some(*blob), None)
            }
            Self::Directory(map) => {
                require_kind(*map, &[MAP_LEAF_KIND, MAP_BRANCH_KIND])?;
                (3, Some(*map), None)
            }
            Self::Conflict(conflict) => {
                require_kind(*conflict, &[CONFLICT_KIND])?;
                (4, Some(*conflict), None)
            }
        };
        let mut descriptor = Descriptor {
            bytes: [0; 35],
            len: 1,
        };
        descriptor.bytes[0] = tag;
        if let Some(reference) = reference {
            descriptor.bytes[1..34].copy_from_slice(&reference.to_bytes()?);
            descriptor.len = 34;
        }
        if let Some(executable) = executable {
            descriptor.bytes[34] = u8::from(executable);
            descriptor.len = 35;
        }
        Ok(descriptor)
    }

    /// Encodes a descriptor, including Absent for conflict terms or witnesses.
    pub fn encode(&self) -> Result<Vec<u8>> {
        Ok(self.descriptor()?.as_slice().to_vec())
    }

    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(payload);
        let entry = reader.entry()?;
        reader.finish()?;
        Ok(entry)
    }

    fn stored_binding(&self) -> Result<()> {
        if matches!(self, Self::Absent) {
            return Err(invalid("Absent is not a directory binding"));
        }
        self.descriptor()?;
        Ok(())
    }
}

fn require_kind(id: Id, allowed: &[u8]) -> Result<()> {
    if !allowed.contains(&id.kind) {
        return Err(invalid("wrong reference kind"));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Leaf(Vec<(Vec<u8>, Entry)>),
    Branch {
        count: u32,
        prefix: Vec<u8>,
        terminal: Option<Entry>,
        children: BTreeMap<u8, Id>,
    },
}

impl Node {
    pub fn kind(&self) -> u8 {
        match self {
            Self::Leaf(_) => MAP_LEAF_KIND,
            Self::Branch { .. } => MAP_BRANCH_KIND,
        }
    }

    fn validate(&self) -> Result<()> {
        match self {
            Self::Leaf(bindings) => {
                if bindings.len() > 32 {
                    return Err(invalid("leaf has more than 32 entries"));
                }
                let mut previous: Option<&[u8]> = None;
                for (suffix, entry) in bindings {
                    validate_nibbles(suffix)?;
                    if previous.is_some_and(|previous| previous >= suffix.as_slice()) {
                        return Err(invalid("leaf suffixes must be sorted and unique"));
                    }
                    entry.stored_binding()?;
                    previous = Some(suffix);
                }
            }
            Self::Branch {
                count,
                prefix,
                terminal,
                children,
            } => {
                if *count < 33 {
                    return Err(invalid("branch count must be at least 33"));
                }
                validate_nibbles(prefix)?;
                if children.len() + usize::from(terminal.is_some()) < 2 {
                    return Err(invalid("branch requires at least two outcomes"));
                }
                if let Some(entry) = terminal {
                    entry.stored_binding()?;
                }
                for (nibble, child) in children {
                    if *nibble > 15 {
                        return Err(invalid("child nibble exceeds 15"));
                    }
                    require_kind(*child, &[MAP_LEAF_KIND, MAP_BRANCH_KIND])?;
                }
            }
        }
        Ok(())
    }
}

fn validate_nibbles(nibbles: &[u8]) -> Result<()> {
    if nibbles.len() > MAX_NIBBLES {
        return Err(invalid("nibble sequence exceeds 510"));
    }
    if nibbles.iter().any(|nibble| *nibble > 15) {
        return Err(invalid("nibble exceeds 15"));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub region: u8,
    pub positive: Vec<(Entry, u8)>,
    pub negative: Vec<(Entry, u8)>,
}

impl Conflict {
    fn validate(&self) -> Result<()> {
        if self.region > 1 {
            return Err(invalid("unknown conflict region"));
        }
        if self.positive.is_empty() || self.positive.len() > 64 || self.negative.len() > 64 {
            return Err(invalid("conflict distinct term count out of range"));
        }
        let positive = self.validate_terms(&self.positive)?;
        let negative = self.validate_terms(&self.negative)?;
        if positive + negative > 64 || positive != negative + 1 {
            return Err(invalid(
                "conflict multiplicities exceed 64 or do not sum to one",
            ));
        }
        if self.positive.len() == 1 && self.negative.is_empty() {
            return Err(invalid(
                "resolved expression must not be stored as a conflict",
            ));
        }
        let (mut p, mut n) = (0, 0);
        while p < self.positive.len() && n < self.negative.len() {
            let positive = self.positive[p].0.descriptor()?;
            let negative = self.negative[n].0.descriptor()?;
            match positive.as_slice().cmp(negative.as_slice()) {
                std::cmp::Ordering::Less => p += 1,
                std::cmp::Ordering::Greater => n += 1,
                std::cmp::Ordering::Equal => {
                    return Err(invalid("opposite conflict terms must cancel"));
                }
            }
        }
        Ok(())
    }

    fn validate_terms(&self, terms: &[(Entry, u8)]) -> Result<u16> {
        let mut total = 0;
        let mut previous: Option<Descriptor> = None;
        for (entry, multiplicity) in terms {
            if !(1..=64).contains(multiplicity) {
                return Err(invalid("conflict multiplicity out of range"));
            }
            total += u16::from(*multiplicity);
            if total > 64 {
                return Err(invalid("conflict multiplicity total exceeds 64"));
            }
            if matches!(entry, Entry::Conflict(_)) {
                return Err(invalid("nested conflict term"));
            }
            if self.region == 1 && !matches!(entry, Entry::Directory(_)) {
                return Err(invalid("source-root conflict term must be a directory"));
            }
            let descriptor = entry.descriptor()?;
            if previous
                .as_ref()
                .is_some_and(|previous| previous.as_slice() >= descriptor.as_slice())
            {
                return Err(invalid(
                    "conflict terms must be descriptor-sorted and unique",
                ));
            }
            previous = Some(descriptor);
        }
        Ok(total)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    pub root: Id,
    pub parents: Vec<Id>,
    pub change: [u8; 16],
    pub author: String,
    pub message: String,
    pub time_ns: i64,
    pub timezone_minutes: i16,
}

impl Revision {
    fn validate(&self) -> Result<()> {
        require_kind(self.root, &[MAP_LEAF_KIND, MAP_BRANCH_KIND, CONFLICT_KIND])?;
        if self.parents.len() > 16 {
            return Err(invalid("revision has more than 16 parents"));
        }
        for (index, parent) in self.parents.iter().enumerate() {
            require_kind(*parent, &[REVISION_KIND])?;
            if self.parents[..index].contains(parent) {
                return Err(invalid("duplicate revision parent"));
            }
        }
        if self.author.len() > MAX_AUTHOR || self.message.len() > MAX_MESSAGE {
            return Err(invalid("revision text exceeds field cap"));
        }
        if !(-1439..=1439).contains(&self.timezone_minutes) {
            return Err(invalid("timezone offset out of range"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Object {
    Blob(Vec<u8>),
    Map(Node),
    Conflict(Conflict),
    Revision(Revision),
}

/// An allocation/work bound, not a format restriction on Blob size. The default
/// permits 64 MiB payloads; callers handling larger blobs must opt in explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeLimits {
    pub max_payload_bytes: usize,
}

impl Default for DecodeLimits {
    fn default() -> Self {
        Self {
            max_payload_bytes: 64 * 1024 * 1024,
        }
    }
}

impl Object {
    pub fn kind(&self) -> u8 {
        match self {
            Self::Blob(_) => BLOB_KIND,
            Self::Map(node) => node.kind(),
            Self::Conflict(_) => CONFLICT_KIND,
            Self::Revision(_) => REVISION_KIND,
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        // Validate counts/lengths before allocating output or narrowing integers.
        match self {
            Self::Blob(bytes) => return Ok(bytes.clone()),
            Self::Map(node) => node.validate()?,
            Self::Conflict(conflict) => conflict.validate()?,
            Self::Revision(revision) => revision.validate()?,
        }
        let mut out = Vec::new();
        match self {
            Self::Blob(_) => unreachable!("blobs return before structured encoding"),
            Self::Map(Node::Leaf(bindings)) => {
                out.push(bindings.len() as u8);
                for (suffix, entry) in bindings {
                    write_nibbles(&mut out, suffix);
                    write_entry(&mut out, entry)?;
                }
            }
            Self::Map(Node::Branch {
                count,
                prefix,
                terminal,
                children,
            }) => {
                out.extend_from_slice(&count.to_le_bytes());
                write_nibbles(&mut out, prefix);
                out.push(u8::from(terminal.is_some()));
                if let Some(entry) = terminal {
                    write_entry(&mut out, entry)?;
                }
                let bitmap = children
                    .keys()
                    .fold(0u16, |bitmap, nibble| bitmap | (1u16 << nibble));
                out.extend_from_slice(&bitmap.to_le_bytes());
                for child in children.values() {
                    out.extend_from_slice(&child.to_bytes()?);
                }
            }
            Self::Conflict(conflict) => {
                out.push(conflict.region);
                for terms in [&conflict.positive, &conflict.negative] {
                    out.push(terms.len() as u8);
                    for (entry, multiplicity) in terms {
                        out.push(*multiplicity);
                        write_entry(&mut out, entry)?;
                    }
                }
            }
            Self::Revision(revision) => {
                out.extend_from_slice(&revision.root.to_bytes()?);
                out.push(revision.parents.len() as u8);
                for parent in &revision.parents {
                    out.extend_from_slice(&parent.to_bytes()?);
                }
                out.extend_from_slice(&revision.change);
                write_text(&mut out, &revision.author);
                write_text(&mut out, &revision.message);
                out.extend_from_slice(&revision.time_ns.to_le_bytes());
                out.extend_from_slice(&revision.timezone_minutes.to_le_bytes());
            }
        }
        Ok(out)
    }

    pub fn id(&self) -> Result<Id> {
        match self {
            Self::Blob(bytes) => Id::hash(BLOB_KIND, bytes),
            _ => Id::hash(self.kind(), &self.encode()?),
        }
    }

    /// Checks the typed hash, local grammar, and exact canonical re-encoding.
    /// Does not fetch references or establish validity in map/root context.
    pub fn decode(id: Id, payload: &[u8]) -> Result<Self> {
        Self::decode_with_limits(id, payload, DecodeLimits::default())
    }

    pub fn decode_with_limits(id: Id, payload: &[u8], limits: DecodeLimits) -> Result<Self> {
        validate_kind(id.kind)?;
        if payload.len() > limits.max_payload_bytes {
            return Err(Error::Limit("object payload bytes"));
        }
        if Id::hash(id.kind, payload)? != id {
            return Err(invalid("object hash mismatch"));
        }
        // Blob re-encoding is the identity, so avoid an extra full-sized copy.
        if id.kind == BLOB_KIND {
            return Ok(Self::Blob(payload.to_vec()));
        }
        let mut reader = Reader::new(payload);
        let object = match id.kind {
            MAP_LEAF_KIND => {
                let count = reader.u8()?;
                if count > 32 {
                    return Err(invalid("leaf has more than 32 entries"));
                }
                let mut bindings = Vec::with_capacity(usize::from(count));
                for _ in 0..count {
                    bindings.push((reader.nibbles()?, reader.entry()?));
                }
                Self::Map(Node::Leaf(bindings))
            }
            MAP_BRANCH_KIND => {
                let count = reader.u32()?;
                if count < 33 {
                    return Err(invalid("branch count must be at least 33"));
                }
                let prefix = reader.nibbles()?;
                let terminal = if reader.boolean()? {
                    Some(reader.entry()?)
                } else {
                    None
                };
                let bitmap = reader.u16()?;
                if bitmap.count_ones() + u32::from(terminal.is_some()) < 2 {
                    return Err(invalid("branch requires at least two outcomes"));
                }
                let mut children = BTreeMap::new();
                for nibble in 0..16 {
                    if bitmap & (1 << nibble) != 0 {
                        children.insert(nibble, reader.reference()?);
                    }
                }
                Self::Map(Node::Branch {
                    count,
                    prefix,
                    terminal,
                    children,
                })
            }
            CONFLICT_KIND => {
                let region = reader.u8()?;
                if region > 1 {
                    return Err(invalid("unknown conflict region"));
                }
                let positive = reader.terms(true)?;
                let negative = reader.terms(false)?;
                Self::Conflict(Conflict {
                    region,
                    positive,
                    negative,
                })
            }
            REVISION_KIND => {
                let root = reader.reference()?;
                let count = reader.u8()?;
                if count > 16 {
                    return Err(invalid("revision has more than 16 parents"));
                }
                let mut parents = Vec::with_capacity(usize::from(count));
                for _ in 0..count {
                    parents.push(reader.reference()?);
                }
                Self::Revision(Revision {
                    root,
                    parents,
                    change: reader.array()?,
                    author: reader.text(MAX_AUTHOR)?,
                    message: reader.text(MAX_MESSAGE)?,
                    time_ns: i64::from_le_bytes(reader.array()?),
                    timezone_minutes: i16::from_le_bytes(reader.array()?),
                })
            }
            _ => return Err(invalid("unknown object kind")),
        };
        reader.finish()?;
        if object.encode()? != payload {
            return Err(invalid("noncanonical object encoding"));
        }
        Ok(object)
    }
}

fn write_entry(out: &mut Vec<u8>, entry: &Entry) -> Result<()> {
    out.extend_from_slice(entry.descriptor()?.as_slice());
    Ok(())
}

fn write_nibbles(out: &mut Vec<u8>, nibbles: &[u8]) {
    out.extend_from_slice(&(nibbles.len() as u16).to_le_bytes());
    for pair in nibbles.chunks(2) {
        out.push((pair[0] << 4) | pair.get(1).copied().unwrap_or(0));
    }
}

fn write_text(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.extend_from_slice(text.as_bytes());
}

struct Reader<'a> {
    remaining: &'a [u8],
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { remaining: bytes }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        if length > self.remaining.len() {
            return Err(invalid("truncated payload"));
        }
        let (bytes, remaining) = self.remaining.split_at(length);
        self.remaining = remaining;
        Ok(bytes)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut bytes = [0; N];
        bytes.copy_from_slice(self.take(N)?);
        Ok(bytes)
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

    fn boolean(&mut self) -> Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(invalid("boolean must be zero or one")),
        }
    }

    fn reference(&mut self) -> Result<Id> {
        Id::from_bytes(self.take(33)?)
    }

    fn entry(&mut self) -> Result<Entry> {
        let entry = match self.u8()? {
            0 => Entry::Absent,
            1 => Entry::File {
                blob: self.reference()?,
                executable: self.boolean()?,
            },
            2 => Entry::Symlink(self.reference()?),
            3 => Entry::Directory(self.reference()?),
            4 => Entry::Conflict(self.reference()?),
            _ => return Err(invalid("unknown entry tag")),
        };
        entry.descriptor()?;
        Ok(entry)
    }

    fn nibbles(&mut self) -> Result<Vec<u8>> {
        let count = usize::from(self.u16()?);
        if count > MAX_NIBBLES {
            return Err(invalid("nibble sequence exceeds 510"));
        }
        let packed = self.take(count.div_ceil(2))?;
        if count % 2 != 0 && packed.last().is_some_and(|last| last & 0x0f != 0) {
            return Err(invalid("nonzero nibble padding"));
        }
        let mut nibbles = Vec::with_capacity(count);
        for index in 0..count {
            let byte = packed[index / 2];
            nibbles.push(if index % 2 == 0 { byte >> 4 } else { byte & 15 });
        }
        Ok(nibbles)
    }

    fn terms(&mut self, positive: bool) -> Result<Vec<(Entry, u8)>> {
        let count = self.u8()?;
        if count > 64 || (positive && count == 0) {
            return Err(invalid("conflict distinct term count out of range"));
        }
        let mut terms = Vec::with_capacity(usize::from(count));
        let mut total = 0u16;
        for _ in 0..count {
            let multiplicity = self.u8()?;
            if !(1..=64).contains(&multiplicity) {
                return Err(invalid("conflict multiplicity out of range"));
            }
            total += u16::from(multiplicity);
            if total > 64 {
                return Err(invalid("conflict multiplicity total exceeds 64"));
            }
            terms.push((self.entry()?, multiplicity));
        }
        Ok(terms)
    }

    fn text(&mut self, cap: usize) -> Result<String> {
        let length = usize::try_from(self.u32()?).map_err(|_| invalid("text length overflow"))?;
        if length > cap {
            return Err(invalid("revision text exceeds field cap"));
        }
        let text = std::str::from_utf8(self.take(length)?)
            .map_err(|_| invalid("revision text is not UTF-8"))?;
        Ok(text.to_owned())
    }

    fn finish(&self) -> Result<()> {
        if self.remaining.is_empty() {
            Ok(())
        } else {
            Err(invalid("trailing payload bytes"))
        }
    }
}

/// Validates one raw-byte source basename, independent of platform restrictions.
pub fn validate_name(name: &[u8]) -> Result<()> {
    if name.is_empty() || name.len() > 255 {
        return Err(invalid("basename length must be 1..255 bytes"));
    }
    if name == b"." || name == b".." || name.iter().any(|byte| matches!(byte, 0 | b'/')) {
        return Err(invalid("invalid basename"));
    }
    Ok(())
}

/// An empty component list denotes the root; otherwise separators count toward
/// the 4096-byte path limit. Does not apply filesystem/materialization policy.
pub fn validate_path<P: AsRef<[u8]>>(components: &[P]) -> Result<()> {
    if components.len() > 256 {
        return Err(invalid("path exceeds 256 components"));
    }
    let mut length = components.len().saturating_sub(1);
    for component in components {
        let component = component.as_ref();
        validate_name(component)?;
        length += component.len();
        if length > 4096 {
            return Err(invalid("path exceeds 4096 bytes"));
        }
    }
    Ok(())
}
