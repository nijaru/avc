//! Explicit, existing filesystem leaf observation; never a scan or database writer.
//!
//! Only regular files and symlink leaves are supported (Linux and macOS). All
//! directory components are opened without following symlinks, including those
//! of the absolute observation root. `.avc` and `.git` components are refused in
//! the entire request before any filesystem access (ASCII case variants too,
//! to protect case-insensitive filesystems). Source bytes are read once;
//! unselected source entries are retained. Missing source directory parents are
//! constructed, but existing non-directory parents and selected source directories
//! refuse rather than implicitly removing unselected source. Requests contain
//! 1..=1024 leaves, at most 64 KiB of `component.len() + 1` bytes in total, and
//! an absolute root pathname of at most 4096 bytes. Captured leaf payloads are at
//! most 16 MiB minus four bytes each and 256 MiB in total. The selected closure
//! is verified at 100,000 objects, 256 MiB total and 16 MiB per canonical payload.
//!
//! Successful observations detect *observed* changes, not all possible races or a
//! simultaneous snapshot. Each leaf's pre-open lstat, open fstat, post-read fstat
//! and post-read lstat must agree in device, inode, full mode, size, mtime and
//! ctime (including nanoseconds). Parent/root name-to-handle identity and mode
//! are guarded before/after reads; directory size/times may change independently.
//! An adversary can still relocate a directory between guards or restore names.
//! On macOS readlinkat uses the guarded parent/name, while Linux reads the pinned
//! symlink descriptor. Neither approach promises a globally atomic snapshot.
//!
//! Durable evidence format v1 (all integers little-endian, at most 256 KiB):
//! `AVCOBS01`, root pathname (u32 length + raw bytes), root-chain count u32,
//! root-chain stats (starting with `/`, ending with the root fd), path count u32,
//! then per path: component count u32, each raw component (u32 length + bytes),
//! relative-parent count u32 and stats in prefix order, leaf kind u8 (1=file,
//! 2=symlink), leaf stat, observed byte count u64. A stat is nine 64-bit words:
//! device u64, inode u64, mode u64 (includes type), size i64, mtime i64,
//! mtime-nanoseconds i64, ctime i64, ctime-nanoseconds i64, link-count u64.
//! Leaf stats encode once the exact equal witnesses at all four checks;
//! parent stats describe opening,
//! with only identity/mode equality asserted by later guards. Root fd device and
//! inode are thus in the last root-chain stat. Paths are lexicographically sorted.
//! Regular-file hardlinks (link count other than one) are unsupported, to avoid
//! ordinary aliases of SQLite-owned files. Physical root canonicalization rejects
//! administrative components before opening descriptors. This does not protect
//! against hostile same-user hardlink substitution or administrative replacement.
//! Payload bytes are NOT evidence metadata; they belong to immutable source blobs.

use crate::{Id, engine::Engine, repository::Result};
use std::path::Path;

pub struct Observation {
    pub root: Id,
    pub witnesses: Vec<u8>,
}

/// Observe a bounded, explicit set of existing leaves without changing live files
/// or persisting anything. On refusal, unpublished immutable engine objects may
/// remain, but no observation root is returned. No deletion is inferred.
pub fn observe(
    engine: &mut Engine,
    base: Id,
    directory: &Path,
    paths: &[Vec<Vec<u8>>],
) -> Result<Observation> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        unix::observe(engine, base, directory, paths)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (engine, base, directory, paths);
        Err(crate::repository::Error::Invalid(
            "observation refused: unsupported platform (requires Linux or macOS dirfds)".into(),
        ))
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix {
    use super::*;
    use crate::{
        Entry, Object,
        engine::{Edit, Limits},
        repository::{Error, MAX_CLOSURE_BYTES, MAX_OBJECT_BYTES, MAX_OBJECTS},
    };
    use rustix::fs::{self, AtFlags, FileType, Mode, OFlags};
    use std::{
        ffi::OsStr,
        fs::File,
        io::Read,
        os::fd::{AsFd, OwnedFd},
        os::unix::ffi::OsStrExt,
    };

    const MAX_PATHS: usize = 1024;
    const MAX_PATH_BYTES: usize = 64 * 1024;
    const MAX_WITNESSES: usize = 256 * 1024;
    // Conservative capture limit, reserving four bytes within the payload cap.
    const MAX_LEAF_BYTES: usize = MAX_OBJECT_BYTES - 4;

    fn refused(context: &str, reason: impl std::fmt::Display) -> Error {
        Error::Invalid(format!("observation refused at {context}: {reason}"))
    }
    fn checked<T, E: std::fmt::Display>(
        context: &str,
        operation: &str,
        result: std::result::Result<T, E>,
    ) -> Result<T> {
        result.map_err(|error| refused(context, format_args!("{operation}: {error}")))
    }
    fn administrative(name: &[u8]) -> bool {
        name.eq_ignore_ascii_case(b".avc") || name.eq_ignore_ascii_case(b".git")
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

    struct Evidence(Vec<u8>);
    impl Evidence {
        fn append(&mut self, bytes: &[u8]) -> Result<()> {
            if bytes.len() > MAX_WITNESSES - self.0.len() {
                return Err(refused("witnesses", "metadata exceeds 256 KiB"));
            }
            self.0.extend_from_slice(bytes);
            Ok(())
        }
        fn count(&mut self, count: usize) -> Result<()> {
            let count = checked("witnesses", "length", u32::try_from(count))?;
            self.append(&count.to_le_bytes())
        }
        fn bytes(&mut self, bytes: &[u8]) -> Result<()> {
            self.count(bytes.len())?;
            self.append(bytes)
        }
    }

    fn root_components(directory: &Path) -> Result<Vec<&[u8]>> {
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
            if administrative(name) {
                return Err(refused("root", "administrative basename .avc or .git"));
            }
        }
        Ok(components)
    }

    struct Request<'a> {
        root_components: Vec<&'a [u8]>,
        paths: Vec<&'a crate::engine::Path>,
    }

    // Preflight the WHOLE request, including administrative names, before open.
    fn preflight<'a>(directory: &'a Path, paths: &'a [Vec<Vec<u8>>]) -> Result<Request<'a>> {
        let components = root_components(directory)?;
        if paths.is_empty() || paths.len() > MAX_PATHS {
            return Err(refused("paths", "requires 1..=1024 explicit leaves"));
        }
        let mut bytes = 0usize;
        for (index, path) in paths.iter().enumerate() {
            // Reject unbounded public input before formatting its raw contents.
            checked(
                &format!("path[{index}]"),
                "path",
                crate::validate_path(path),
            )?;
            let context = format!("{path:?}");
            if path.is_empty() || path.iter().any(|name| administrative(name)) {
                return Err(refused(
                    &context,
                    "empty path or administrative basename .avc or .git",
                ));
            }
            bytes += path.iter().map(|name| name.len() + 1).sum::<usize>();
            if bytes > MAX_PATH_BYTES {
                return Err(refused("paths", "raw path bytes exceed 64 KiB"));
            }
        }
        let mut paths: Vec<_> = paths.iter().collect();
        paths.sort_unstable();
        for pair in paths.windows(2) {
            if pair[1].starts_with(pair[0]) {
                return Err(refused("paths", "duplicate or overlapping leaf paths"));
            }
        }
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
        if size > MAX_LEAF_BYTES || size > remaining {
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

    pub(super) fn observe(
        engine: &mut Engine,
        base: Id,
        directory: &Path,
        paths: &[Vec<Vec<u8>>],
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
        let mut evidence = Evidence(Vec::new());
        evidence.append(b"AVCOBS01")?;
        evidence.bytes(directory.as_os_str().as_bytes())?;
        evidence.count(chain.len())?;
        for directory in &chain {
            evidence.append(&directory.opened.0)?;
        }
        evidence.count(request.paths.len())?;
        let mut root = base;
        let mut captured = 0;
        let mut previous_parents: &[Vec<u8>] = &[];
        for path in request.paths {
            let context = format!("{path:?}");
            let parents = &path[..path.len() - 1];
            let shared = parents
                .iter()
                .zip(previous_parents)
                .take_while(|(a, b)| a == b)
                .count();
            // Sorted paths keep shared directory prefixes contiguous. Retain
            // their handles so replacement between sibling reads is detected.
            chain.truncate(root_depth + shared);
            for name in &parents[shared..] {
                let parent = &chain.last().expect("root directory present").fd;
                chain.push(open_directory(parent, name, &context)?);
            }
            guard(&chain, &context)?;
            evidence.count(path.len())?;
            for name in path {
                evidence.bytes(name)?;
            }
            evidence.count(chain.len() - root_depth)?;
            for parent in &chain[root_depth..] {
                evidence.append(&parent.opened.0)?;
            }
            let parent = &chain.last().expect("root directory present").fd;
            let (bytes, symlink, executable, witness) = leaf(
                parent,
                path.last().expect("nonempty validated path"),
                &context,
                MAX_CLOSURE_BYTES - captured,
            )?;
            guard(&chain, &context)?;
            evidence.append(&[if symlink { 2 } else { 1 }])?;
            evidence.append(&witness.0)?;
            evidence.append(&(bytes.len() as u64).to_le_bytes())?;
            captured += bytes.len();
            // Explicitly construct only missing logical parents; never replace
            // an existing source ancestor or remove unselected directory entries.
            for length in 1..path.len() {
                let prefix = &path[..length];
                match checked(&context, "read source parent", engine.read(root, prefix))? {
                    Entry::Directory(_) => {}
                    Entry::Absent => {
                        let empty = engine.empty()?;
                        root = put(engine, root, prefix, Entry::Directory(empty))?;
                    }
                    _ => return Err(refused(&context, "source parent is not a directory")),
                }
            }
            if matches!(engine.read(root, path)?, Entry::Directory(_)) {
                return Err(refused(
                    &context,
                    "selected source is a directory; implicit deletion refused",
                ));
            }
            let blob = engine.blob(bytes)?;
            let value = if symlink {
                Entry::Symlink(blob)
            } else {
                Entry::File { blob, executable }
            };
            root = put(engine, root, path, value)?;
            previous_parents = parents;
        }
        guard(&chain, "root/final parent")?;
        // Engine verification bounds the complete selected closure. Its API has
        // no per-object limit, so check encoded payloads in that closure too.
        engine.verify(
            root,
            Limits {
                objects: MAX_OBJECTS,
                bindings: 1_000_000,
                bytes: MAX_CLOSURE_BYTES,
            },
        )?;
        verify_payloads(engine, root)?;
        Ok(Observation {
            root,
            witnesses: evidence.0,
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
        fn leaf_remaining_byte_budget_refuses_before_reading() {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            std::fs::write(root.join("file"), b"eightbyt").unwrap();
            let parent = open_directory(fs::CWD, root.as_os_str().as_bytes(), "test").unwrap();
            assert!(leaf(&parent.fd, b"file", "test", 4).is_err());
        }
    }
}
