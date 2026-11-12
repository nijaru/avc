//! Read-only, bounded Linux/macOS filesystem observation, never database custody.
//!
//! Leaves observes explicit files/links; Recursive also enumerates explicit
//! directories (including an empty path for the root). Traversal uses anchored,
//! NOFOLLOW directory descriptors and raw basenames. Administrative `.avc`/`.git`
//! names (ASCII case-insensitive) and directory identity aliases are unobserved:
//! no descendant is opened, and existing excluded Source bindings are preserved.
//! Missing ordinary Source bindings refuse unless RecursiveRemovingMissing
//! observes their absence in a fully scanned existing parent. Explicit missing
//! scopes/parents and directory-to-leaf changes always refuse. Non-directory
//! Source ancestors refuse partial selection.
//!
//! Requests: 1..=1024 disjoint paths, 64 KiB total component lengths plus one per
//! component, canonical paths <=256 components/4096 bytes, names <=255 bytes.
//! Expanded entries: <=100,000 (observations, preserved exclusions and absences),
//! 16 MiB per Blob, 256 MiB captured bytes. The entire resulting closure,
//! including unselected Base, is checked at
//! 100,000 objects/256 MiB/16 MiB per payload. Metadata has MAX_WITNESS_BYTES limit.
//!
//! Evidence detects observed races, NOT a simultaneous or complete filesystem
//! snapshot. File/link pre-open lstat, open fstat, post-read fstat/lstat agree.
//! Scanned directories have equal full stats before enumeration and after their
//! subtree; ancestor/root guards compare identity and mode. Name-to-fd checks
//! surround reads. Relocation/restoration between guards can escape detection.
//! macOS readlinkat uses guarded parent/name; Linux uses the pinned link fd.
//! Regular files with nlink != 1 refuse before opening (including ordinary
//! hardlink aliases of SQLite-owned files). Trusted root/admin replacement by a
//! hostile same-user actor is outside this contract. No database file is opened.
//!
//! Private witness v3 grammar (all integers little-endian; strict EOF):
//! `AVCOBS03`, mode u8 (0=Leaves, 1=Recursive, 2=RecursiveRemovingMissing), raw
//! absolute root (u32 length + bytes), root-chain count u32, that many 72-byte
//! stats (from `/` through root), files/symlinks/scanned-directories/removed counts
//! (four u64), record count u32, then records. Each record is path (u32 component
//! count, each u32 length + raw bytes), kind u8, then kind-specific data:
//! 0=unscanned ancestor: stat; 1=file or 2=link: stat + observed length u64;
//! 3=scanned directory: stat + u32 raw child-name count + each u32 length/raw name
//! (sorted, unique, including excluded live names, from enumeration through EOF);
//! 4=UNOBSERVED exclusion, 5=absent old binding: no further data. Kind 5 requires
//! mode 2, a nonempty nonadministrative path, and a scanned immediate parent whose
//! full name list lacks that raw basename. Records follow sorted-scope,
//! sorted-live-child depth-first traversal; Base-only exclusions/absences follow
//! that directory's live children. Ancestor records may repeat. Exclusions also
//! include preserved administrative Base-only bindings. Full before/after scanned
//! stats cover membership, not atomic or global snapshot consistency.
//! A stat is device u64, inode u64, full mode u64, size i64, mtime i64,
//! mtime-nanoseconds i64, ctime i64, ctime-nanoseconds i64, nlink u64.
//! Equal checks are encoded once; ancestor stats assert only identity/mode.
//! Counts describe actual observations, not preserved excluded Source entries.
//! `report` validates framing and derives diagnostics, not Source authority.
//! Payload bytes belong only to Native Blobs and are never repeated in evidence.

use crate::{Id, engine::Engine, repository::Result};
use std::path::Path;

pub const MAX_WITNESS_BYTES: usize = 16 * 1024 * 1024 - 4096;
const MAX_ENTRIES: usize = 100_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum CaptureMode {
    Leaves = 0,
    Recursive = 1,
    RecursiveRemovingMissing = 2,
}

impl TryFrom<u8> for CaptureMode {
    type Error = crate::repository::Error;

    fn try_from(value: u8) -> Result<Self> {
        match value {
            0 => Ok(Self::Leaves),
            1 => Ok(Self::Recursive),
            2 => Ok(Self::RecursiveRemovingMissing),
            _ => Err(refused("mode", "unknown capture mode")),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct CaptureReport {
    pub files: u64,
    pub symlinks: u64,
    pub directories: u64,
    pub excluded: Vec<Vec<Vec<u8>>>,
    pub removed: Vec<Vec<Vec<u8>>>,
}

fn refused(context: &str, reason: impl std::fmt::Display) -> crate::repository::Error {
    crate::repository::Error::Invalid(format!("observation refused at {context}: {reason}"))
}
fn administrative(name: &[u8]) -> bool {
    name.eq_ignore_ascii_case(b".avc") || name.eq_ignore_ascii_case(b".git")
}

/// Validate the entire scope without filesystem access. The caller may retain
/// its own sorted representation; this function owns the shared scope policy.
pub(crate) fn validate_paths(paths: &[Vec<Vec<u8>>], mode: CaptureMode) -> Result<()> {
    if paths.is_empty() || paths.len() > 1024 {
        return Err(refused("paths", "requires 1..=1024 explicit paths"));
    }
    let mut bytes = 0;
    for path in paths {
        crate::validate_path(path)?;
        if (path.is_empty() && mode == CaptureMode::Leaves)
            || path.iter().any(|name| administrative(name))
        {
            return Err(refused(
                "paths",
                "empty leaf path or administrative basename .avc or .git",
            ));
        }
        bytes += path.iter().map(|name| name.len() + 1).sum::<usize>();
        if bytes > 64 * 1024 {
            return Err(refused("paths", "raw path bytes exceed 64 KiB"));
        }
    }
    let mut sorted: Vec<_> = paths.iter().collect();
    sorted.sort_unstable();
    if sorted.windows(2).any(|pair| pair[1].starts_with(pair[0])) {
        return Err(refused("paths", "duplicate or overlapping paths"));
    }
    Ok(())
}

pub struct Observation {
    pub root: Id,
    pub witnesses: Vec<u8>,
}

/// Observe explicit leaves or recursive directory scopes without changing live
/// files or persisting anything. On refusal, unpublished immutable engine objects may
/// remain, but no observation root is returned. Only mode 2 may prune old bindings,
/// using complete parent enumeration and successful directory guards.
pub fn observe(
    engine: &mut Engine,
    base: Id,
    directory: &Path,
    paths: &[Vec<Vec<u8>>],
    mode: CaptureMode,
) -> Result<Observation> {
    validate_paths(paths, mode)?;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        unix::observe(engine, base, directory, paths, mode)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (engine, base, directory, paths, mode);
        Err(crate::repository::Error::Invalid(
            "observation refused: unsupported platform (requires Linux or macOS dirfds)".into(),
        ))
    }
}

/// Open an explicitly supplied regular byte input, not a source observation.
/// The explicit input may be a symlink; resolve it first, then anchor its actual
/// parents and refuse SQLite administration files, hardlinks and special files
/// before opening the payload. Explicit exports within `.avc/exports` remain
/// readable: file identity, not the whole administration subtree, is protected.
/// The caller owns read-size bounds; this does not promise stable contents or an
/// atomic filesystem snapshot.
pub fn open_input(path: &Path) -> Result<std::fs::File> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        unix::open_input(path)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = path;
        Err(refused(
            "input",
            "requires Linux or macOS directory descriptors",
        ))
    }
}

/// Parse bounded private evidence for post-publication diagnostics. Damaged or
/// unknown evidence is an error, never a reason to undo an acknowledged capture.
pub fn report(witnesses: &[u8]) -> Result<CaptureReport> {
    struct Reader<'a>(&'a [u8]);
    impl<'a> Reader<'a> {
        fn take(&mut self, n: usize) -> Result<&'a [u8]> {
            let (head, rest) = self
                .0
                .split_at_checked(n)
                .ok_or_else(|| refused("witnesses", "truncated metadata"))?;
            self.0 = rest;
            Ok(head)
        }
        fn count(&mut self) -> Result<usize> {
            Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("four bytes")) as usize)
        }
        fn word(&mut self) -> Result<u64> {
            Ok(u64::from_le_bytes(
                self.take(8)?.try_into().expect("eight bytes"),
            ))
        }
        fn raw(&mut self, max: usize) -> Result<&'a [u8]> {
            let n = self.count()?;
            if n > max {
                return Err(refused("witnesses", "oversized field"));
            }
            self.take(n)
        }
        fn stat(&mut self, kind: u8) -> Result<u64> {
            let stat = self.take(72)?;
            let word = |offset| {
                u64::from_le_bytes(stat[offset..offset + 8].try_into().expect("stat word"))
            };
            let expected = match kind {
                0 | 3 => 0o040000,
                1 => 0o100000,
                2 => 0o120000,
                _ => unreachable!(),
            };
            if word(16) > u64::from(u32::MAX)
                || word(16) & 0o170000 != expected
                || word(24) > i64::MAX as u64
                || word(40) >= 1_000_000_000
                || word(56) >= 1_000_000_000
                || (kind == 1 && word(64) != 1)
            {
                return Err(refused("witnesses", "invalid stat fields"));
            }
            Ok(word(24))
        }
    }
    if witnesses.len() > MAX_WITNESS_BYTES {
        return Err(refused("witnesses", "metadata limit"));
    }
    let mut input = Reader(witnesses);
    if input.take(8)? != b"AVCOBS03" {
        return Err(refused("witnesses", "unknown version"));
    }
    let mode = CaptureMode::try_from(input.take(1)?[0])?;
    let root = input.raw(4096)?;
    if !root.starts_with(b"/") || root.contains(&0) {
        return Err(refused("witnesses", "invalid root"));
    }
    for name in root.split(|b| *b == b'/').filter(|name| !name.is_empty()) {
        crate::validate_name(name)?;
        if administrative(name) {
            return Err(refused("witnesses", "administrative root"));
        }
    }
    let chain = input.count()?;
    if chain
        != 1 + root
            .split(|b| *b == b'/')
            .filter(|name| !name.is_empty())
            .count()
    {
        return Err(refused("witnesses", "invalid root chain"));
    }
    for _ in 0..chain {
        input.stat(0)?;
    }
    let expected = [input.word()?, input.word()?, input.word()?, input.word()?];
    let records = input.count()?;
    if records > MAX_WITNESS_BYTES / 5 {
        return Err(refused("witnesses", "record limit"));
    }
    let mut result = CaptureReport {
        files: 0,
        symlinks: 0,
        directories: 0,
        excluded: Vec::new(),
        removed: Vec::new(),
    };
    // Borrow raw names from the bounded input, never allocate payloads or consult
    // Source. The retained BeforeRoot, not this diagnostic parser, owns old values.
    let mut scanned = std::collections::BTreeMap::new();
    let mut absent = std::collections::BTreeSet::new();
    let mut observed = std::collections::BTreeSet::new();
    let mut bindings = std::collections::BTreeSet::new();
    let mut expanded_names = 0;
    let mut payload_bytes = 0u64;
    for _ in 0..records {
        let count = input.count()?;
        if count > 256 || count * 5 + 1 > input.0.len() {
            return Err(refused("witnesses", "path depth limit or truncated path"));
        }
        let mut path = Vec::with_capacity(count);
        let mut bytes = 0;
        for _ in 0..count {
            let name = input.raw(255)?;
            crate::validate_name(name)?;
            bytes += name.len() + 1;
            path.push(name);
        }
        if bytes.saturating_sub(1) > 4096 {
            return Err(refused("witnesses", "path byte limit"));
        }
        let kind = input.take(1)?[0];
        if kind != 4 && path.iter().any(|name| administrative(name)) {
            return Err(refused("witnesses", "observed administrative path"));
        }
        if kind <= 3 {
            observed.insert(path.clone());
        }
        if (1..=5).contains(&kind) && !bindings.insert(path.clone()) {
            return Err(refused("witnesses", "duplicate binding record"));
        }
        match kind {
            0 => {
                input.stat(kind)?;
            }
            3 => {
                if mode == CaptureMode::Leaves {
                    return Err(refused("witnesses", "scanned directory in leaf mode"));
                }
                input.stat(kind)?;
                let count = input.count()?;
                if count > MAX_ENTRIES - expanded_names || count > input.0.len() / 5 {
                    return Err(refused("witnesses", "raw name limit or truncated names"));
                }
                expanded_names += count;
                let mut names = Vec::with_capacity(count);
                for _ in 0..count {
                    let name = input.raw(255)?;
                    crate::validate_name(name)?;
                    if path.len() == 256 || bytes + name.len() > 4096 {
                        return Err(refused("witnesses", "expanded path limit"));
                    }
                    if names.last().is_some_and(|previous| *previous >= name) {
                        return Err(refused("witnesses", "unsorted or duplicate raw names"));
                    }
                    names.push(name);
                }
                if scanned.insert(path, names).is_some() {
                    return Err(refused("witnesses", "duplicate scanned directory"));
                }
                result.directories += 1;
            }
            1 | 2 => {
                if path.is_empty() {
                    return Err(refused("witnesses", "empty leaf"));
                }
                let size = input.stat(kind)?;
                let observed = input.word()?;
                if size != observed || size > 16 * 1024 * 1024 {
                    return Err(refused("witnesses", "invalid observed size"));
                }
                payload_bytes += size;
                if payload_bytes > 256 * 1024 * 1024 {
                    return Err(refused("witnesses", "payload byte limit"));
                }
                if kind == 1 {
                    result.files += 1;
                } else {
                    result.symlinks += 1;
                }
            }
            4 => {
                if path.is_empty() {
                    return Err(refused("witnesses", "empty exclusion"));
                }
                result
                    .excluded
                    .push(path.into_iter().map(<[u8]>::to_vec).collect());
            }
            5 => {
                if mode != CaptureMode::RecursiveRemovingMissing || path.is_empty() {
                    return Err(refused(
                        "witnesses",
                        "absence requires mode 2 and nonempty path",
                    ));
                }
                if expanded_names == MAX_ENTRIES || !absent.insert(path) {
                    return Err(refused(
                        "witnesses",
                        "duplicate or excessive absent bindings",
                    ));
                }
                expanded_names += 1;
            }
            _ => return Err(refused("witnesses", "unknown record kind")),
        }
        if result.files
            + result.symlinks
            + result.directories
            + absent.len() as u64
            + result.excluded.len() as u64
            > MAX_ENTRIES as u64
        {
            return Err(refused("witnesses", "expanded entry limit"));
        }
    }
    for path in &observed {
        for length in 1..=path.len() {
            let (name, parent) = path[..length].split_last().expect("nonempty prefix");
            if let Some(names) = scanned.get(parent)
                && names.binary_search(name).is_err()
            {
                return Err(refused(
                    "witnesses",
                    "observed binding or ancestor absent from raw name list",
                ));
            }
            if absent.contains(&path[..length]) {
                return Err(refused(
                    "witnesses",
                    "observed path contradicts absent binding",
                ));
            }
        }
    }
    for path in &bindings {
        for length in 1..path.len() {
            if absent.contains(&path[..length]) {
                return Err(refused("witnesses", "binding beneath an absent ancestor"));
            }
        }
    }
    for path in &absent {
        let (name, parent) = path.split_last().expect("nonempty absent path");
        let names = scanned
            .get(parent)
            .ok_or_else(|| refused("witnesses", "absent binding lacks scanned parent"))?;
        if names.binary_search(name).is_ok() {
            return Err(refused(
                "witnesses",
                "absent binding present in raw name list",
            ));
        }
        result
            .removed
            .push(path.iter().map(|name| name.to_vec()).collect());
    }
    if !input.0.is_empty()
        || expected
            != [
                result.files,
                result.symlinks,
                result.directories,
                result.removed.len() as u64,
            ]
    {
        return Err(refused(
            "witnesses",
            "trailing bytes or inconsistent counts",
        ));
    }
    Ok(result)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix {
    use super::*;
    use crate::{
        Entry, Object,
        engine::{Edit, Limits},
        repository::{MAX_CLOSURE_BYTES, MAX_OBJECT_BYTES, MAX_OBJECTS},
    };
    use rustix::fs::{self, AtFlags, FileType, Mode, OFlags};
    use std::{
        ffi::OsStr,
        fs::File,
        io::Read,
        os::fd::{AsFd, OwnedFd},
        os::unix::ffi::OsStrExt,
    };

    fn checked<T, E: std::fmt::Display>(
        context: &str,
        operation: &str,
        result: std::result::Result<T, E>,
    ) -> Result<T> {
        result.map_err(|error| refused(context, format_args!("{operation}: {error}")))
    }

    // Convert explicitly instead of casting: stat field widths/signedness vary
    // between Unix ABIs. The durable representation never silently truncates.
    #[derive(Clone, Debug, Eq, PartialEq)]
    struct Stat([u8; 72]);
    impl Stat {
        fn from(raw: fs::Stat, context: &str) -> Result<Self> {
            let mut bytes = [0; 72];
            let unsigned = [
                checked(
                    context,
                    "device representation",
                    u64::try_from(i128::from(raw.st_dev)),
                )?,
                checked(
                    context,
                    "inode representation",
                    u64::try_from(i128::from(raw.st_ino)),
                )?,
                u64::from(raw.st_mode),
            ];
            for (slot, value) in bytes.as_chunks_mut::<8>().0.iter_mut().zip(unsigned) {
                slot.copy_from_slice(&value.to_le_bytes());
            }
            let signed = [
                i128::from(raw.st_size),
                i128::from(raw.st_mtime),
                i128::from(raw.st_mtime_nsec),
                i128::from(raw.st_ctime),
                i128::from(raw.st_ctime_nsec),
            ];
            for (slot, value) in bytes[24..64].as_chunks_mut::<8>().0.iter_mut().zip(signed) {
                let value = checked(context, "stat representation", i64::try_from(value))?;
                slot.copy_from_slice(&value.to_le_bytes());
            }
            let links = checked(
                context,
                "link count representation",
                u64::try_from(i128::from(raw.st_nlink)),
            )?;
            bytes[64..].copy_from_slice(&links.to_le_bytes());
            Ok(Self(bytes))
        }
        fn same_directory(&self, other: &Self) -> bool {
            self.0[..24] == other.0[..24]
        }
    }
    fn stat(fd: impl AsFd, context: &str) -> Result<Stat> {
        Stat::from(checked(context, "fstat", fs::fstat(fd))?, context)
    }
    fn lstat(fd: impl AsFd, name: &OsStr, context: &str) -> Result<fs::Stat> {
        checked(
            context,
            "lstat",
            fs::statat(fd, name, AtFlags::SYMLINK_NOFOLLOW),
        )
    }
    fn require_same(before: &Stat, after: &Stat, context: &str) -> Result<()> {
        if before != after {
            return Err(refused(context, "leaf changed during observation"));
        }
        Ok(())
    }

    struct Directory {
        fd: OwnedFd,
        name: Vec<u8>,
        opened: Stat,
    }
    fn open_directory(parent: impl AsFd, name: &[u8], context: &str) -> Result<Directory> {
        let directory = directory_handle(&parent, name, context)?;
        // Directory descriptors do not own SQLite file locks. Check identity
        // before any descendant file is opened, including normalized aliases.
        if administrative_identity(parent, &directory.opened, context)? {
            return Err(refused(context, "administrative directory identity"));
        }
        Ok(directory)
    }
    fn directory_handle(parent: impl AsFd, name: &[u8], context: &str) -> Result<Directory> {
        let name_os = OsStr::from_bytes(name);
        let before = Stat::from(lstat(&parent, name_os, context)?, context)?;
        let fd = checked(
            context,
            "open directory without symlinks",
            fs::openat(
                &parent,
                name_os,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            ),
        )?;
        let opened = stat(&fd, context)?;
        let after = Stat::from(lstat(parent, name_os, context)?, context)?;
        if !before.same_directory(&opened) || !after.same_directory(&opened) {
            return Err(refused(context, "directory changed while opening"));
        }
        Ok(Directory {
            fd,
            name: name.to_vec(),
            opened,
        })
    }
    fn administrative_identity(parent: impl AsFd, before: &Stat, context: &str) -> Result<bool> {
        for admin in [".avc", ".git"] {
            match fs::statat(&parent, admin, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(raw) if before.same_directory(&Stat::from(raw, context)?) => return Ok(true),
                Ok(_) | Err(rustix::io::Errno::NOENT) => {}
                Err(error) => {
                    return Err(refused(
                        context,
                        format_args!("check administrative identity: {error}"),
                    ));
                }
            }
        }
        Ok(false)
    }

    fn missing_administrative_alias(parent: &OwnedFd, name: &[u8]) -> Result<bool> {
        let context = format!("missing binding {name:?}");
        match fs::statat(parent, OsStr::from_bytes(name), AtFlags::SYMLINK_NOFOLLOW) {
            Ok(raw) if FileType::from_raw_mode(raw.st_mode).is_dir() => {
                administrative_identity(parent, &Stat::from(raw, &context)?, &context)
            }
            Ok(_) | Err(rustix::io::Errno::NOENT) => Ok(false),
            Err(error) => Err(refused(
                &context,
                format_args!("check administrative alias: {error}"),
            )),
        }
    }

    fn guard(chain: &[Directory], context: &str) -> Result<()> {
        for (index, directory) in chain.iter().enumerate() {
            let named = if index == 0 {
                lstat(fs::CWD, OsStr::new("/"), context)?
            } else {
                lstat(
                    &chain[index - 1].fd,
                    OsStr::from_bytes(&directory.name),
                    context,
                )?
            };
            let named = Stat::from(named, context)?;
            if !directory.opened.same_directory(&named)
                || !directory
                    .opened
                    .same_directory(&stat(&directory.fd, context)?)
            {
                return Err(refused(context, "root/parent name or handle changed"));
            }
        }
        Ok(())
    }

    struct Evidence {
        bytes: Vec<u8>,
        reserved: usize,
        records: u32,
        counts: [u64; 4],
    }
    impl Evidence {
        fn reserve(&mut self, length: usize) -> Result<()> {
            if length > MAX_WITNESS_BYTES - self.bytes.len() - self.reserved {
                return Err(refused("witnesses", "metadata exceeds MAX_WITNESS_BYTES"));
            }
            self.reserved += length;
            Ok(())
        }
        fn append(&mut self, bytes: &[u8]) -> Result<()> {
            self.reserve(bytes.len())?;
            self.reserved -= bytes.len();
            self.bytes.extend_from_slice(bytes);
            Ok(())
        }
        fn count(&mut self, count: usize) -> Result<()> {
            let count = checked("witnesses", "length", u32::try_from(count))?;
            self.append(&count.to_le_bytes())
        }
        fn raw(&mut self, bytes: &[u8]) -> Result<()> {
            self.count(bytes.len())?;
            self.append(bytes)
        }
        fn record(
            &mut self,
            path: &[Vec<u8>],
            kind: u8,
            stat: Option<&Stat>,
            size: usize,
        ) -> Result<()> {
            // Check the whole record before growing the output buffer.
            let length = record_size(path, kind);
            self.reserve(length)?;
            self.reserved -= length;
            self.count(path.len())?;
            for name in path {
                self.raw(name)?;
            }
            self.append(&[kind])?;
            if let Some(stat) = stat {
                self.append(&stat.0)?;
            }
            if kind == 1 || kind == 2 {
                self.append(&(size as u64).to_le_bytes())?;
            }
            self.records += 1;
            if (1..=3).contains(&kind) {
                self.counts[usize::from(kind - 1)] += 1;
            } else if kind == 5 {
                self.counts[3] += 1;
            }
            Ok(())
        }
    }
    fn record_size(path: &[Vec<u8>], kind: u8) -> usize {
        5 + path.iter().map(|name| 4 + name.len()).sum::<usize>()
            + match kind {
                0 | 3 => 72,
                1 | 2 => 80,
                _ => 0,
            }
    }

    fn directory_components(directory: &Path) -> Result<Vec<&[u8]>> {
        let raw = directory.as_os_str().as_bytes();
        if !directory.is_absolute() || raw.len() > 4096 || raw.contains(&0) {
            return Err(refused(
                "root",
                "requires an absolute pathname of at most 4096 bytes",
            ));
        }
        let components: Vec<_> = raw
            .split(|b| *b == b'/')
            .filter(|c| !c.is_empty())
            .collect();
        for name in &components {
            checked("root", "basename", crate::validate_name(name))?;
        }
        Ok(components)
    }
    fn root_components(directory: &Path) -> Result<Vec<&[u8]>> {
        let components = directory_components(directory)?;
        if components.iter().any(|name| administrative(name)) {
            return Err(refused("root", "administrative basename .avc or .git"));
        }
        Ok(components)
    }

    pub(super) fn open_input(path: &Path) -> Result<File> {
        let context = format!("input {path:?}");
        let physical = checked(&context, "resolve explicit input", path.canonicalize())?;
        let parent = physical
            .parent()
            .ok_or_else(|| refused(&context, "input has no parent"))?;
        let name = physical
            .file_name()
            .ok_or_else(|| refused(&context, "input has no basename"))?
            .as_bytes();
        crate::validate_name(name)?;
        let components = directory_components(parent)?;
        let mut chain = vec![directory_handle(fs::CWD, b"/", &context)?];
        let mut administrations = Vec::new();
        for name in components {
            let parent = &chain.last().expect("root present").fd;
            let directory = directory_handle(parent, name, &context)?;
            let native = match fs::statat(parent, ".avc", AtFlags::SYMLINK_NOFOLLOW) {
                Ok(raw) => directory.opened.0[..16] == Stat::from(raw, &context)?.0[..16],
                Err(rustix::io::Errno::NOENT) => false,
                Err(error) => {
                    return Err(refused(
                        &context,
                        format_args!("check native administration: {error}"),
                    ));
                }
            };
            chain.push(directory);
            if native || name.eq_ignore_ascii_case(b".avc") {
                administrations.push(chain.len() - 1);
            }
        }
        guard(&chain, &context)?;
        let directory = &chain.last().expect("root present").fd;
        let name = OsStr::from_bytes(name);
        let raw = lstat(directory, name, &context)?;
        if !FileType::from_raw_mode(raw.st_mode).is_file() {
            return Err(refused(
                &context,
                "byte input must be an ordinary regular file",
            ));
        }
        if raw.st_nlink != 1 {
            return Err(refused(&context, "hardlinked byte inputs are unsupported"));
        }
        let before = Stat::from(raw, &context)?;
        // Explicit byte suppliers may live below administration (e.g. exports),
        // but no descriptor may independently open a SQLite-owned inode.
        for index in administrations {
            for resource in ["store.sqlite", "store.sqlite-wal", "store.sqlite-shm"] {
                match fs::statat(&chain[index].fd, resource, AtFlags::SYMLINK_NOFOLLOW) {
                    Ok(raw) if before.0[..16] == Stat::from(raw, &context)?.0[..16] => {
                        return Err(refused(&context, "SQLite administration byte input"));
                    }
                    Ok(_) | Err(rustix::io::Errno::NOENT) => {}
                    Err(error) => {
                        return Err(refused(
                            &context,
                            format_args!("check SQLite resource identity: {error}"),
                        ));
                    }
                }
            }
        }
        let fd = checked(
            &context,
            "open byte input",
            fs::openat(
                directory,
                name,
                OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            ),
        )?;
        require_same(&before, &stat(&fd, &context)?, &context)?;
        require_same(
            &before,
            &Stat::from(lstat(directory, name, &context)?, &context)?,
            &context,
        )?;
        guard(&chain, &context)?;
        Ok(File::from(fd))
    }

    struct Request<'a> {
        root_components: Vec<&'a [u8]>,
        paths: Vec<&'a crate::engine::Path>,
    }

    // Preflight the WHOLE request, including administrative names, before open.
    fn preflight<'a>(directory: &'a Path, paths: &'a [Vec<Vec<u8>>]) -> Result<Request<'a>> {
        let components = root_components(directory)?;
        let mut paths: Vec<_> = paths.iter().collect();
        paths.sort_unstable();
        Ok(Request {
            root_components: components,
            paths,
        })
    }

    fn leaf(
        parent: &OwnedFd,
        name: &[u8],
        context: &str,
        remaining: usize,
    ) -> Result<(Vec<u8>, bool, bool, Stat)> {
        let name = OsStr::from_bytes(name);
        let raw = lstat(parent, name, context)?;
        let kind = FileType::from_raw_mode(raw.st_mode);
        let symlink = kind.is_symlink();
        if !kind.is_file() && !symlink {
            return Err(refused(
                context,
                "requires an existing regular file or symlink leaf",
            ));
        }
        if kind.is_file() && raw.st_nlink != 1 {
            return Err(refused(context, "hardlinked regular files are unsupported"));
        }
        let size = checked(
            context,
            "nonnegative file size",
            usize::try_from(raw.st_size),
        )?;
        if size > MAX_OBJECT_BYTES || size > remaining {
            return Err(refused(
                context,
                "leaf or total captured-byte limit exceeded",
            ));
        }
        let executable = raw.st_mode & 0o100 != 0;
        let before = Stat::from(raw, context)?;
        let flags = if symlink {
            #[cfg(target_os = "linux")]
            {
                OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC
            }
            #[cfg(target_os = "macos")]
            {
                // Darwin rejects O_NOFOLLOW even with O_SYMLINK. O_SYMLINK
                // opens the link itself, never its target (open(2)); NONBLOCK
                // also prevents a substituted FIFO from hanging before fstat.
                OFlags::RDONLY | OFlags::SYMLINK | OFlags::NONBLOCK | OFlags::CLOEXEC
            }
        } else {
            // A FIFO substituted after lstat must not block before fstat rejects it.
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC
        };
        let fd = checked(
            context,
            "open leaf without following symlinks",
            fs::openat(parent, name, flags, Mode::empty()),
        )?;
        let opened_raw = checked(context, "fstat opened leaf", fs::fstat(&fd))?;
        let opened_kind = FileType::from_raw_mode(opened_raw.st_mode);
        if (!symlink && !opened_kind.is_file()) || (symlink && !opened_kind.is_symlink()) {
            return Err(refused(context, "leaf type changed while opening"));
        }
        require_same(&before, &Stat::from(opened_raw, context)?, context)?;
        let mut file = File::from(fd);
        let bytes = if symlink {
            // One bounded readlink call. An extra byte detects growth/truncation;
            // no retry could combine different live versions of the target.
            let mut bytes = vec![0; size + 1];
            #[cfg(target_os = "linux")]
            let length = checked(
                context,
                "read symlink",
                fs::readlinkat_raw(&file, "", bytes.as_mut_slice()),
            )?;
            #[cfg(target_os = "macos")]
            let length = checked(
                context,
                "read symlink",
                fs::readlinkat_raw(parent, name, bytes.as_mut_slice()),
            )?;
            if length != size {
                return Err(refused(
                    context,
                    "symlink size changed or unsupported target size",
                ));
            }
            bytes.truncate(length);
            bytes
        } else {
            let mut bytes = vec![0; size];
            checked(context, "read file", file.read_exact(&mut bytes))?;
            let mut extra = [0];
            if checked(context, "check file EOF", file.read(&mut extra))? != 0 {
                return Err(refused(context, "file grew during observation"));
            }
            bytes
        };
        require_same(&before, &stat(&file, context)?, context)?;
        require_same(
            &before,
            &Stat::from(lstat(parent, name, context)?, context)?,
            context,
        )?;
        Ok((bytes, symlink, executable, before))
    }

    fn put(engine: &mut Engine, root: Id, path: &[Vec<u8>], value: Entry) -> Result<Id> {
        let context = format!("{path:?}");
        let expected = checked(&context, "read source", engine.read(root, path))?;
        checked(
            &context,
            "edit source",
            engine.edit(
                root,
                &[Edit {
                    path: path.to_vec(),
                    expected,
                    value,
                }],
            ),
        )
    }

    struct Scanner<'a> {
        engine: &'a mut Engine,
        evidence: Evidence,
        captured: usize,
        expanded: usize,
        mode: CaptureMode,
        // Only pruned old directories need this metadata-only walk. Sharing a
        // directory at many logical paths must not repeatedly traverse its DAG.
        checked_prunable: std::collections::BTreeSet<Id>,
    }
    struct Frame {
        path: crate::engine::Path,
        names: std::vec::IntoIter<(Vec<u8>, usize)>,
        entries: Vec<(Vec<u8>, Entry)>,
        prior: std::collections::BTreeMap<Vec<u8>, Entry>,
        before: Stat,
    }
    impl Scanner<'_> {
        fn entry_budget(&mut self) -> Result<()> {
            if self.expanded == MAX_ENTRIES {
                return Err(refused("directory", "expanded entry limit exceeds 100000"));
            }
            self.expanded += 1;
            Ok(())
        }
        fn read_leaf(
            &mut self,
            chain: &[Directory],
            path: &[Vec<u8>],
            prior: &Entry,
        ) -> Result<Entry> {
            let context = format!("{path:?}");
            if matches!(prior, Entry::Directory(_)) {
                return Err(refused(
                    &context,
                    "selected source is a directory; implicit deletion refused",
                ));
            }
            if self.mode != CaptureMode::Leaves && matches!(prior, Entry::Conflict(_)) {
                return Err(refused(
                    &context,
                    "recursive conflict replacement is unsupported",
                ));
            }
            guard(chain, &context)?;
            // Charge evidence before allocating/reading payload bytes.
            let reserve = record_size(path, 1);
            self.evidence.reserve(reserve)?;
            let (bytes, symlink, executable, witness) = leaf(
                &chain.last().expect("root directory present").fd,
                path.last().expect("leaf path is nonempty"),
                &context,
                MAX_CLOSURE_BYTES - self.captured,
            )?;
            guard(chain, &context)?;
            self.evidence.reserved -= reserve;
            self.evidence.record(
                path,
                if symlink { 2 } else { 1 },
                Some(&witness),
                bytes.len(),
            )?;
            self.captured += bytes.len();
            let blob = self.engine.blob(bytes)?;
            Ok(if symlink {
                Entry::Symlink(blob)
            } else {
                Entry::File { blob, executable }
            })
        }
        fn frame(
            &mut self,
            chain: &[Directory],
            path: crate::engine::Path,
            prior: Entry,
        ) -> Result<Frame> {
            let context = format!("{path:?}");
            let prior = match prior {
                Entry::Directory(id) => self.engine.list(id, &[])?.into_iter().collect(),
                Entry::Conflict(_) => {
                    return Err(refused(
                        &context,
                        "recursive conflict replacement is unsupported",
                    ));
                }
                _ => std::collections::BTreeMap::new(),
            };
            guard(chain, &context)?;
            let fd = &chain.last().expect("root directory present").fd;
            let before = stat(fd, &context)?;
            self.evidence.record(&path, 3, Some(&before), 0)?;
            self.evidence.reserve(4)?; // full raw-name count, before enumeration
            let mut names = Vec::new();
            let mut dir = checked(&context, "enumerate directory", fs::Dir::read_from(fd))?;
            for item in &mut dir {
                let item = checked(&context, "read directory entry", item)?;
                let name = item.file_name().to_bytes();
                if name == b"." || name == b".." {
                    continue;
                }
                self.entry_budget()?;
                crate::validate_name(name)?;
                let length = path.iter().map(|name| name.len() + 1).sum::<usize>() + name.len();
                if path.len() == 256 || length > 4096 {
                    return Err(refused(
                        &context,
                        "expanded path exceeds 256 components or 4096 bytes",
                    ));
                }
                let reserve = record_size(&path, 1) + 4 + name.len();
                // Charge both the child record and the duplicate name in this
                // directory's full enumeration BEFORE copying untrusted bytes.
                self.evidence.reserve(reserve + 4 + name.len())?;
                names.push((name.to_vec(), reserve));
            }
            names.sort_unstable_by(|a, b| a.0.cmp(&b.0));
            if names.windows(2).any(|pair| pair[0].0 == pair[1].0) {
                return Err(refused(&context, "duplicate enumerated directory name"));
            }
            self.evidence.reserved -= 4;
            self.evidence.count(names.len())?;
            for (name, _) in &names {
                self.evidence.reserved -= 4 + name.len();
                self.evidence.raw(name)?;
            }
            Ok(Frame {
                path,
                names: names.into_iter(),
                entries: Vec::new(),
                prior,
                before,
            })
        }
        fn require_prunable(&mut self, entry: &Entry) -> Result<()> {
            let mut pending = Vec::new();
            match entry {
                Entry::Conflict(_) => {
                    return Err(refused(
                        "missing binding",
                        "conflict alternatives are protected",
                    ));
                }
                Entry::Directory(id) => pending.push(*id),
                _ => {}
            }
            while let Some(id) = pending.pop() {
                if !self.checked_prunable.insert(id) {
                    continue;
                }
                // Engine.list reads only directory metadata; never open Blob
                // payloads or interpret opaque conflict alternatives as absent.
                for (name, entry) in self.engine.list(id, &[])? {
                    if administrative(&name) {
                        return Err(refused(
                            "missing directory",
                            "contains protected administrative binding",
                        ));
                    }
                    match entry {
                        Entry::Conflict(_) => {
                            return Err(refused(
                                "missing directory",
                                "conflict alternatives are protected",
                            ));
                        }
                        Entry::Directory(child) => pending.push(child),
                        _ => {}
                    }
                }
            }
            Ok(())
        }
        fn subtree(
            &mut self,
            chain: &mut Vec<Directory>,
            path: crate::engine::Path,
            prior: Entry,
        ) -> Result<Entry> {
            // One frame and descriptor per directory depth; no Rust recursion.
            let mut stack = vec![self.frame(chain, path, prior)?];
            loop {
                let frame = stack.last_mut().expect("active scan frame");
                if let Some((name, reserve)) = frame.names.next() {
                    let mut path = frame.path.clone();
                    path.push(name.clone());
                    self.evidence.reserved -= reserve;
                    let context = format!("{path:?}");
                    let prior = frame.prior.remove(&name).unwrap_or(Entry::Absent);
                    let fd = &chain.last().expect("scan directory present").fd;
                    // Never stat/open a descendant of an excluded basename.
                    let excluded = if administrative(&name) {
                        true
                    } else {
                        let raw = lstat(fd, OsStr::from_bytes(&name), &context)?;
                        FileType::from_raw_mode(raw.st_mode).is_dir()
                            && administrative_identity(fd, &Stat::from(raw, &context)?, &context)?
                    };
                    if excluded {
                        self.evidence.record(&path, 4, None, 0)?;
                        if prior != Entry::Absent {
                            frame.entries.push((name, prior));
                        }
                        continue;
                    }
                    guard(chain, &context)?;
                    let raw = lstat(
                        &chain.last().expect("scan directory present").fd,
                        OsStr::from_bytes(&name),
                        &context,
                    )?;
                    if FileType::from_raw_mode(raw.st_mode).is_dir() {
                        let child = open_directory(
                            &chain.last().expect("scan directory present").fd,
                            &name,
                            &context,
                        )?;
                        chain.push(child);
                        stack.push(self.frame(chain, path, prior)?);
                    } else {
                        let value = self.read_leaf(chain, &path, &prior)?;
                        stack
                            .last_mut()
                            .expect("active scan frame")
                            .entries
                            .push((name, value));
                    }
                    continue;
                }
                let mut frame = stack.pop().expect("active scan frame");
                for (name, entry) in frame.prior {
                    // Absence is RAW-name absence, not a failed OS lookup. Only
                    // administrative aliases get a special metadata-only lookup.
                    let fd = &chain.last().expect("scan directory present").fd;
                    let excluded = administrative(&name)
                        || (self.mode == CaptureMode::RecursiveRemovingMissing
                            && missing_administrative_alias(fd, &name)?);
                    let kind = if excluded {
                        4
                    } else if self.mode == CaptureMode::RecursiveRemovingMissing {
                        self.require_prunable(&entry)?;
                        5
                    } else {
                        let mut missing = frame.path.clone();
                        missing.push(name);
                        return Err(refused(
                            &format!("{missing:?}"),
                            "missing previously-selected path; deletion unsupported",
                        ));
                    };
                    // Every Base-only output binding consumes an expansion
                    // slot too; live exclusions were charged during enumeration.
                    self.entry_budget()?;
                    let reserve = record_size(&frame.path, kind) + 4 + name.len();
                    self.evidence.reserve(reserve)?;
                    let mut path = frame.path.clone();
                    path.push(name.clone());
                    self.evidence.reserved -= reserve;
                    self.evidence.record(&path, kind, None, 0)?;
                    if excluded {
                        frame.entries.push((name, entry));
                    }
                }
                let context = format!("{:?}", frame.path);
                guard(chain, &context)?;
                let directory = chain.last().expect("scan directory present");
                require_directory_same(&frame.before, &stat(&directory.fd, &context)?, &context)?;
                let named = if chain.len() == 1 {
                    lstat(fs::CWD, OsStr::new("/"), &context)?
                } else {
                    lstat(
                        &chain[chain.len() - 2].fd,
                        OsStr::from_bytes(&directory.name),
                        &context,
                    )?
                };
                require_directory_same(&frame.before, &Stat::from(named, &context)?, &context)?;
                let value = Entry::Directory(self.engine.directory(frame.entries)?);
                if let Some(parent) = stack.last_mut() {
                    let name = frame.path.pop().expect("child directory has basename");
                    parent.entries.push((name, value));
                    chain.pop();
                } else {
                    return Ok(value);
                }
            }
        }
    }
    fn require_directory_same(before: &Stat, after: &Stat, context: &str) -> Result<()> {
        if before != after {
            return Err(refused(
                context,
                "enumerated directory changed during observation",
            ));
        }
        Ok(())
    }

    pub(super) fn observe(
        engine: &mut Engine,
        base: Id,
        directory: &Path,
        paths: &[Vec<Vec<u8>>],
        mode: CaptureMode,
    ) -> Result<Observation> {
        let request = preflight(directory, paths)?;
        // Resolve only the explicitly trusted root to exclude physical admin
        // ancestors. Descendant selections never pass through canonicalize.
        let physical = checked(
            &format!("root {directory:?}"),
            "canonicalize root",
            directory.canonicalize(),
        )?;
        root_components(&physical)?;
        // Still open the ORIGINAL path componentwise, refusing root symlinks.
        let mut chain = vec![open_directory(fs::CWD, b"/", "root")?];
        for name in request.root_components {
            let parent = &chain.last().expect("root directory present").fd;
            chain.push(open_directory(parent, name, "root")?);
        }
        let root_depth = chain.len();
        guard(&chain, "root")?;
        let mut evidence = Evidence {
            bytes: Vec::new(),
            reserved: 0,
            records: 0,
            counts: [0; 4],
        };
        evidence.append(b"AVCOBS03")?;
        evidence.append(&[mode as u8])?;
        evidence.raw(directory.as_os_str().as_bytes())?;
        evidence.count(chain.len())?;
        for directory in &chain {
            evidence.append(&directory.opened.0)?;
        }
        let report_offset = evidence.bytes.len();
        evidence.append(&[0; 36])?; // four u64 counts and one u32 record count
        let mut scanner = Scanner {
            engine,
            evidence,
            captured: 0,
            expanded: 0,
            mode,
            checked_prunable: std::collections::BTreeSet::new(),
        };
        let mut root = base;
        let mut previous_parents: &[Vec<u8>] = &[];
        for path in request.paths {
            let context = format!("{path:?}");
            let parents = &path[..path.len().saturating_sub(1)];
            // Construct only missing logical ancestors, never replace one.
            for length in 1..path.len() {
                let prefix = &path[..length];
                match checked(
                    &context,
                    "read source parent",
                    scanner.engine.read(root, prefix),
                )? {
                    Entry::Directory(_) => {}
                    Entry::Absent => {
                        let empty = scanner.engine.empty()?;
                        root = put(scanner.engine, root, prefix, Entry::Directory(empty))?;
                    }
                    _ => {
                        return Err(refused(
                            &context,
                            "source parent is not a directory; select the ancestor recursively",
                        ));
                    }
                }
            }
            let shared = parents
                .iter()
                .zip(previous_parents)
                .take_while(|(a, b)| a == b)
                .count();
            chain.truncate(root_depth + shared);
            for (index, name) in parents.iter().enumerate().skip(shared) {
                let parent = &chain.last().expect("root directory present").fd;
                let directory = open_directory(parent, name, &context)?;
                scanner
                    .evidence
                    .record(&parents[..=index], 0, Some(&directory.opened), 0)?;
                chain.push(directory);
            }
            guard(&chain, &context)?;
            scanner.entry_budget()?;
            let prior = scanner.engine.read(root, path)?;
            let is_directory = if path.is_empty() {
                true
            } else {
                let named = fs::statat(
                    &chain.last().expect("root directory present").fd,
                    OsStr::from_bytes(path.last().expect("nonempty path")),
                    AtFlags::SYMLINK_NOFOLLOW,
                );
                match named {
                    Ok(raw) => FileType::from_raw_mode(raw.st_mode).is_dir(),
                    Err(rustix::io::Errno::NOENT)
                        if mode != CaptureMode::Leaves && prior != Entry::Absent =>
                    {
                        let reason = if mode == CaptureMode::RecursiveRemovingMissing {
                            "missing previously-selected path; deletion unsupported for explicit scopes; select an existing parent recursively"
                        } else {
                            "missing previously-selected path; deletion unsupported"
                        };
                        return Err(refused(&context, reason));
                    }
                    Err(error) => return Err(refused(&context, format_args!("lstat: {error}"))),
                }
            };
            let value = if mode != CaptureMode::Leaves && is_directory {
                if let Some(name) = path.last() {
                    let child = open_directory(
                        &chain.last().expect("root directory present").fd,
                        name,
                        &context,
                    )?;
                    chain.push(child);
                }
                let value = scanner.subtree(&mut chain, path.clone(), prior)?;
                if !path.is_empty() {
                    chain.pop();
                }
                value
            } else {
                scanner.read_leaf(&chain, path, &prior)?
            };
            root = if path.is_empty() {
                // Engine edits intentionally require nonempty paths. The scanner
                // already built and checked the complete selected root directory.
                match value {
                    Entry::Directory(id) => id,
                    _ => return Err(refused("root", "requires a directory")),
                }
            } else {
                put(scanner.engine, root, path, value)?
            };
            previous_parents = parents;
        }
        guard(&chain, "root/final parent")?;
        for (index, count) in scanner.evidence.counts.iter().enumerate() {
            scanner.evidence.bytes[report_offset + index * 8..report_offset + (index + 1) * 8]
                .copy_from_slice(&count.to_le_bytes());
        }
        scanner.evidence.bytes[report_offset + 32..report_offset + 36]
            .copy_from_slice(&scanner.evidence.records.to_le_bytes());
        // Engine verification bounds the complete selected closure. Its API has
        // no per-object limit, so check encoded payloads in that closure too.
        scanner.engine.verify(
            root,
            Limits {
                objects: MAX_OBJECTS,
                bindings: 1_000_000,
                bytes: MAX_CLOSURE_BYTES,
            },
        )?;
        verify_payloads(scanner.engine, root)?;
        Ok(Observation {
            root,
            witnesses: scanner.evidence.bytes,
        })
    }

    fn verify_payloads(engine: &Engine, root: Id) -> Result<()> {
        use std::collections::BTreeSet;
        let mut seen = BTreeSet::new();
        let mut pending = vec![root];
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            let object = engine.object(id)?;
            let size = match object {
                Object::Blob(bytes) => bytes.len(),
                _ => object.encode()?.len(),
            };
            if size > MAX_OBJECT_BYTES {
                return Err(refused("source closure", "object payload exceeds 16 MiB"));
            }
            let mut entry = |entry: &Entry| match entry {
                Entry::Absent => {}
                Entry::File { blob, .. } | Entry::Symlink(blob) => pending.push(*blob),
                Entry::Directory(id) | Entry::Conflict(id) => pending.push(*id),
            };
            match object {
                Object::Blob(_) => {}
                Object::Map(crate::Node::Leaf(bindings)) => {
                    for (_, value) in bindings {
                        entry(value);
                    }
                }
                Object::Map(crate::Node::Branch {
                    terminal, children, ..
                }) => {
                    if let Some(value) = terminal {
                        entry(value);
                    }
                    pending.extend(children.values());
                }
                Object::Conflict(conflict) => {
                    for (value, _) in conflict.positive.iter().chain(&conflict.negative) {
                        entry(value);
                    }
                }
                Object::Revision(revision) => {
                    pending.push(revision.root);
                    pending.extend(&revision.parents);
                }
            }
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::unix::fs::{PermissionsExt, symlink};

        #[test]
        fn directory_guard_rejects_name_replacement_with_symlink() {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            let mut chain = vec![open_directory(fs::CWD, b"/", "test").unwrap()];
            for name in root_components(&root).unwrap() {
                let directory = open_directory(&chain.last().unwrap().fd, name, "test").unwrap();
                chain.push(directory);
            }
            std::fs::create_dir(root.join("parent")).unwrap();
            chain.push(open_directory(&chain.last().unwrap().fd, b"parent", "test").unwrap());
            guard(&chain, "test").unwrap();
            std::fs::rename(root.join("parent"), root.join("moved")).unwrap();
            symlink(&root, root.join("parent")).unwrap();
            assert!(guard(&chain, "test").is_err());
        }

        #[test]
        fn leaf_witness_detects_same_size_edit_and_mode_change() {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            let path = root.join("file");
            std::fs::write(&path, b"before").unwrap();
            let file = File::open(&path).unwrap();
            let before = stat(&file, "test").unwrap();
            std::fs::write(&path, b"after!").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            assert!(require_same(&before, &stat(&file, "test").unwrap(), "test").is_err());
        }

        #[test]
        fn enumerated_membership_guard_detects_directory_change() {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            let file = File::open(&root).unwrap();
            file.set_modified(std::time::UNIX_EPOCH).unwrap();
            let before = stat(&file, "test").unwrap();
            std::fs::write(root.join("new-member"), b"member").unwrap();
            let after = stat(&file, "test").unwrap();
            // Identity guards alone deliberately allow independent membership
            // changes; enumerated-directory guards must reject them.
            assert!(before.same_directory(&after));
            assert!(require_directory_same(&before, &after, "test").is_err());
        }

        #[test]
        fn metadata_reservations_and_expansion_are_bounded_before_allocation() {
            let mut engine = Engine::new();
            let mut scanner = Scanner {
                engine: &mut engine,
                evidence: Evidence {
                    bytes: Vec::new(),
                    reserved: 0,
                    records: 0,
                    counts: [0; 4],
                },
                captured: 0,
                expanded: MAX_ENTRIES - 1,
                mode: CaptureMode::Leaves,
                checked_prunable: std::collections::BTreeSet::new(),
            };
            scanner.entry_budget().unwrap();
            assert!(scanner.entry_budget().is_err());
            scanner.evidence.reserve(MAX_WITNESS_BYTES).unwrap();
            assert!(scanner.evidence.append(b"x").is_err());
            assert!(scanner.evidence.bytes.is_empty());
            scanner.evidence.reserved -= 1;
            scanner.evidence.append(b"x").unwrap();
            assert_eq!(scanner.evidence.bytes, b"x");
            assert!(
                scanner
                    .evidence
                    .record(&[], 3, Some(&Stat([0; 72])), 0)
                    .is_err()
            );
            assert_eq!(scanner.evidence.bytes, b"x");
        }

        #[test]
        fn base_only_exclusions_consume_the_shared_entry_budget() {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            let mut chain = vec![open_directory(fs::CWD, b"/", "test").unwrap()];
            for name in root_components(&root).unwrap() {
                let directory = open_directory(&chain.last().unwrap().fd, name, "test").unwrap();
                chain.push(directory);
            }
            let mut engine = Engine::new();
            let blob = engine.blob(b"protected".to_vec()).unwrap();
            let base = engine
                .directory(vec![
                    (b".avc".to_vec(), Entry::Symlink(blob)),
                    (b".git".to_vec(), Entry::Symlink(blob)),
                ])
                .unwrap();
            for mode in [
                CaptureMode::Recursive,
                CaptureMode::RecursiveRemovingMissing,
            ] {
                for remaining in [1, 2] {
                    let mut scanner = Scanner {
                        engine: &mut engine,
                        evidence: Evidence {
                            bytes: Vec::new(),
                            reserved: 0,
                            records: 0,
                            counts: [0; 4],
                        },
                        captured: 0,
                        expanded: MAX_ENTRIES - remaining,
                        mode,
                        checked_prunable: std::collections::BTreeSet::new(),
                    };
                    assert_eq!(
                        scanner
                            .subtree(&mut chain, vec![], Entry::Directory(base))
                            .is_err(),
                        remaining == 1
                    );
                    assert_eq!(scanner.expanded, MAX_ENTRIES);
                }
            }
        }

        #[test]
        fn pruned_directory_walk_caches_shared_metadata_ids_without_blob_ids() {
            let mut engine = Engine::new();
            let blob = engine.blob(b"payload".to_vec()).unwrap();
            let leaf = engine
                .directory(vec![(b"file".to_vec(), Entry::Symlink(blob))])
                .unwrap();
            let branch = engine
                .directory(vec![
                    (b"a".to_vec(), Entry::Directory(leaf)),
                    (b"b".to_vec(), Entry::Directory(leaf)),
                ])
                .unwrap();
            let root = engine
                .directory(vec![
                    (b"left".to_vec(), Entry::Directory(branch)),
                    (b"right".to_vec(), Entry::Directory(branch)),
                ])
                .unwrap();
            let mut scanner = Scanner {
                engine: &mut engine,
                evidence: Evidence {
                    bytes: Vec::new(),
                    reserved: 0,
                    records: 0,
                    counts: [0; 4],
                },
                captured: 0,
                expanded: 0,
                mode: CaptureMode::RecursiveRemovingMissing,
                checked_prunable: std::collections::BTreeSet::new(),
            };
            for _ in 0..2 {
                scanner.require_prunable(&Entry::Directory(root)).unwrap();
                assert_eq!(
                    scanner.checked_prunable,
                    [root, branch, leaf].into_iter().collect()
                );
            }
            assert!(!scanner.checked_prunable.contains(&blob));
        }

        #[test]
        fn scanned_raw_name_list_and_child_record_share_the_exact_metadata_budget() {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            std::fs::write(root.join("child"), b"not read by enumeration").unwrap();
            let mut chain = vec![open_directory(fs::CWD, b"/", "test").unwrap()];
            for name in root_components(&root).unwrap() {
                let directory = open_directory(&chain.last().unwrap().fd, name, "test").unwrap();
                chain.push(directory);
            }
            let child_record = record_size(&[], 1) + 4 + b"child".len();
            let full_name_list = 4 + 4 + b"child".len();
            let required = record_size(&[], 3) + full_name_list + child_record;
            let mut engine = Engine::new();
            for short in [0, 1] {
                let mut scanner = Scanner {
                    engine: &mut engine,
                    evidence: Evidence {
                        bytes: Vec::new(),
                        reserved: 0,
                        records: 0,
                        counts: [0; 4],
                    },
                    captured: 0,
                    expanded: 0,
                    mode: CaptureMode::RecursiveRemovingMissing,
                    checked_prunable: std::collections::BTreeSet::new(),
                };
                scanner
                    .evidence
                    .reserve(MAX_WITNESS_BYTES - required + short)
                    .unwrap();
                let result = scanner.frame(&chain, vec![], Entry::Absent);
                assert_eq!(result.is_err(), short == 1);
                assert_eq!(scanner.captured, 0);
                if short == 0 {
                    assert_eq!(result.unwrap().names.len(), 1);
                    assert_eq!(
                        scanner.evidence.bytes.len(),
                        record_size(&[], 3) + full_name_list
                    );
                    assert_eq!(
                        scanner.evidence.bytes.len() + scanner.evidence.reserved,
                        MAX_WITNESS_BYTES
                    );
                }
            }
        }

        #[test]
        fn leaf_remaining_byte_budget_refuses_before_reading() {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            std::fs::write(root.join("file"), b"eightbyt").unwrap();
            let parent = open_directory(fs::CWD, root.as_os_str().as_bytes(), "test").unwrap();
            assert!(leaf(&parent.fd, b"file", "test", 4).is_err());
        }
    }
}
