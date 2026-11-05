//! Experimental source materialization into a fresh directory, not a workspace.
//!
//! The caller retains the immutable source in its repository. Errors leave any
//! incomplete output for inspection; there is no cleanup, journal, or resume
//! protocol. Checked syncs are not qualification for full power-loss custody.

use crate::{Id, engine::Engine, engine::Limits};
use std::{fmt, io, path::Path};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Source(crate::Error),
    UnsupportedPlatform,
    UnsupportedName {
        name: Vec<u8>,
        reason: &'static str,
    },
    UnsupportedSymlink {
        blob: Id,
        reason: &'static str,
    },
    Io {
        operation: &'static str,
        source: io::Error,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(error) => error.fmt(f),
            Self::UnsupportedPlatform => {
                f.write_str("fresh-directory restore requires Unix dirfd operations")
            }
            Self::UnsupportedName { name, reason } => {
                write!(f, "cannot materialize source basename {name:?}: {reason}")
            }
            Self::UnsupportedSymlink { blob, reason } => {
                write!(f, "cannot materialize symlink target {blob}: {reason}")
            }
            Self::Io { operation, source } => write!(f, "restore {operation}: {source}"),
        }
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Source(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
impl From<crate::Error> for Error {
    fn from(error: crate::Error) -> Self {
        Self::Source(error)
    }
}

/// Restore exact source bytes into an exclusively created directory.
///
/// Refuses every existing destination, including empty directories and dangling
/// symlinks. Complete source verification and conflict/representation preflight
/// precede creation. `.avc` is an ordinary source name here: no administration
/// directory or workspace registration is created. Files become 0644 or 0755;
/// directories are private (0700). No other source metadata exists in v1.
///
/// Destination parent components must already exist and must not be symlinks.
/// The caller chooses that parent; subsequent operations use directory handles,
/// never accumulated source paths. No symlink is followed. Handles resist name
/// replacement, but cannot prevent another writer relocating an opened directory
/// or modifying output afterward. This is not a simultaneous filesystem snapshot.
/// Files and directories are synced with checked errors (regular files also use
/// F_FULLFSYNC on macOS); success is not a power-loss custody guarantee.
/// Failure may leave incomplete output, which is never removed automatically.
/// Non-Unix platforms refuse before any writes.
pub fn restore(engine: &Engine, root: Id, destination: &Path) -> Result<()> {
    restore_with_limits(engine, root, destination, Limits::default())
}

/// As [`restore`], with explicit verification and expansion budgets. In addition
/// to bounding verification, `objects` bounds expanded map visits, `bindings`
/// bounds output entries, and `bytes` bounds output bytes (including repeated
/// shared subtrees). Both passes are iterative and do not collect a full tree.
pub fn restore_with_limits(
    engine: &Engine,
    root: Id,
    destination: &Path,
    limits: Limits,
) -> Result<()> {
    #[cfg(unix)]
    {
        unix::restore(engine, root, destination, limits)
    }
    #[cfg(not(unix))]
    {
        let _ = (engine, root, destination, limits);
        Err(Error::UnsupportedPlatform)
    }
}

#[cfg(unix)]
mod unix {
    use super::*;
    use crate::{Entry, Node, Object, validate_name};
    use rustix::fs::{self, Mode, OFlags};
    use std::{
        ffi::OsStr,
        fs::File,
        io::Write,
        os::fd::{AsFd, OwnedFd},
        os::unix::ffi::OsStrExt,
        path::Component,
    };

    fn io<T>(operation: &'static str, result: io::Result<T>) -> Result<T> {
        result.map_err(|source| Error::Io { operation, source })
    }
    fn syscall<T>(operation: &'static str, result: rustix::io::Result<T>) -> Result<T> {
        io(operation, result.map_err(io::Error::from))
    }
    fn invalid(reason: &str) -> Error {
        crate::Error::Invalid(reason.to_owned()).into()
    }
    fn limit(reason: &'static str) -> Error {
        crate::Error::Limit(reason).into()
    }
    fn blob(engine: &Engine, id: Id) -> Result<&[u8]> {
        match engine.object(id)? {
            Object::Blob(bytes) => Ok(bytes),
            _ => Err(invalid("expected source blob")),
        }
    }

    // A cursor per radix node, not a pending task per directory entry. Prefixes
    // are at most 510 nibbles and source directory depth is at most 256; neither
    // directory width nor repeated DAG expansion accumulates an output plan.
    struct Frame<'a> {
        node: &'a Node,
        prefix: Vec<u8>,
        index: usize,
    }
    enum Cursor<'a> {
        Map(Frame<'a>),
        Leave,
    }
    enum Event<'a> {
        Entry(Vec<u8>, &'a Entry),
        Leave,
    }
    enum Step<'a> {
        Entry(Vec<u8>, &'a Entry),
        Child(Id, Vec<u8>),
        Pop,
    }
    struct Walk<'a> {
        engine: &'a Engine,
        stack: Vec<Cursor<'a>>,
        maps: usize,
        entries: usize,
        limits: Limits,
    }
    impl<'a> Walk<'a> {
        fn new(engine: &'a Engine, root: Id, limits: Limits) -> Result<Self> {
            let mut walk = Self {
                engine,
                stack: vec![Cursor::Leave],
                maps: 0,
                entries: 0,
                limits,
            };
            walk.push_map(root, Vec::new())?;
            Ok(walk)
        }
        fn push_map(&mut self, id: Id, mut prefix: Vec<u8>) -> Result<()> {
            if self.maps >= self.limits.objects {
                return Err(limit("restore map visits"));
            }
            self.maps += 1;
            let Object::Map(node) = self.engine.object(id)? else {
                return Err(invalid("expected source directory map"));
            };
            if let Node::Branch { prefix: suffix, .. } = node {
                prefix.extend(suffix);
            }
            if prefix.len() > 510 {
                return Err(invalid("source radix bound"));
            }
            self.stack.push(Cursor::Map(Frame {
                node,
                prefix,
                index: 0,
            }));
            Ok(())
        }
        fn next(&mut self) -> Result<Option<Event<'a>>> {
            loop {
                let Some(cursor) = self.stack.last_mut() else {
                    return Ok(None);
                };
                let Cursor::Map(frame) = cursor else {
                    self.stack.pop();
                    return Ok(Some(Event::Leave));
                };
                let step = match frame.node {
                    Node::Leaf(entries) => match entries.get(frame.index) {
                        Some((suffix, entry)) => {
                            let mut key = frame.prefix.clone();
                            key.extend(suffix);
                            Step::Entry(key, entry)
                        }
                        None => Step::Pop,
                    },
                    Node::Branch {
                        terminal, children, ..
                    } => {
                        if frame.index == 0 {
                            frame.index += 1;
                            if let Some(entry) = terminal {
                                // Terminal keys use exactly the branch prefix.
                                let key = frame.prefix.clone();
                                return self.entry(key, entry).map(Some);
                            }
                        }
                        match children.iter().nth(frame.index - 1) {
                            Some((slot, id)) => {
                                let mut prefix = frame.prefix.clone();
                                prefix.push(*slot);
                                Step::Child(*id, prefix)
                            }
                            None => Step::Pop,
                        }
                    }
                };
                frame.index += 1;
                match step {
                    Step::Pop => {
                        self.stack.pop();
                    }
                    Step::Child(id, prefix) => self.push_map(id, prefix)?,
                    Step::Entry(key, entry) => return self.entry(key, entry).map(Some),
                }
            }
        }
        fn entry(&mut self, key: Vec<u8>, entry: &'a Entry) -> Result<Event<'a>> {
            if self.entries >= self.limits.bindings {
                return Err(limit("restore output entries"));
            }
            self.entries += 1;
            if key.len() > 510 || !key.len().is_multiple_of(2) {
                return Err(invalid("invalid complete source name"));
            }
            let name: Vec<_> = key
                .as_chunks::<2>()
                .0
                .iter()
                .map(|p| p[0] << 4 | p[1])
                .collect();
            validate_name(&name)?;
            match entry {
                Entry::Directory(id) => {
                    self.stack.push(Cursor::Leave);
                    self.push_map(*id, Vec::new())?;
                }
                Entry::Conflict(id) => return Err(crate::Error::Conflict(*id).into()),
                Entry::Absent => return Err(invalid("absent source binding")),
                _ => {}
            }
            Ok(Event::Entry(name, entry))
        }
    }

    fn preflight(engine: &Engine, root: Id, limits: Limits) -> Result<()> {
        engine.verify(root, limits)?;
        match engine.object(root)? {
            Object::Conflict(_) => return Err(crate::Error::Conflict(root).into()),
            Object::Map(_) => {}
            _ => return Err(invalid("restore requires a source root, not a revision")),
        }
        let mut walk = Walk::new(engine, root, limits)?;
        let mut bytes = 0usize;
        while let Some(event) = walk.next()? {
            // Darwin's native filesystems reject ill-formed UTF-8 basenames,
            // though symlink payloads remain opaque bytes. Do not create a
            // partial tree merely to discover this known representation limit.
            #[cfg(target_os = "macos")]
            if let Event::Entry(ref name, _) = event
                && std::str::from_utf8(name).is_err()
            {
                return Err(Error::UnsupportedName {
                    name: name.clone(),
                    reason: "non-UTF-8 basename on macOS",
                });
            }
            if let Event::Entry(_, Entry::File { blob: id, .. } | Entry::Symlink(id)) = event {
                let payload = blob(engine, *id)?;
                bytes = bytes
                    .checked_add(payload.len())
                    .ok_or_else(|| limit("restore output bytes"))?;
                if bytes > limits.bytes {
                    return Err(limit("restore output bytes"));
                }
                if matches!(event, Event::Entry(_, Entry::Symlink(_))) {
                    let reason = if payload.is_empty() {
                        Some("empty target")
                    } else if payload.contains(&0) {
                        Some("NUL in target")
                    }
                    // Kernel pathname-copy limits include the terminating NUL.
                    // Other filesystem-specific create failures remain checked.
                    else if cfg!(target_os = "macos") && payload.len() >= 1024
                        || cfg!(target_os = "linux") && payload.len() >= 4096
                    {
                        Some("target exceeds platform pathname limit")
                    } else {
                        None
                    };
                    if let Some(reason) = reason {
                        return Err(Error::UnsupportedSymlink { blob: *id, reason });
                    }
                }
            }
        }
        Ok(())
    }

    fn open_dir(parent: impl AsFd, name: &OsStr) -> Result<OwnedFd> {
        syscall(
            "open directory without following symlinks",
            fs::openat(
                parent,
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            ),
        )
    }
    fn new_dir(parent: impl AsFd, name: &OsStr) -> Result<OwnedFd> {
        syscall(
            "create exclusive directory",
            fs::mkdirat(&parent, name, Mode::RUSR | Mode::WUSR | Mode::XUSR),
        )?;
        open_dir(parent, name)
    }
    fn parent(destination: &Path) -> Result<(OwnedFd, &OsStr)> {
        let Some(Component::Normal(name)) = destination.components().next_back() else {
            return Err(invalid("destination must name a fresh directory"));
        };
        let path = destination
            .parent()
            .ok_or_else(|| invalid("destination has no parent"))?;
        let mut fd = open_dir(
            fs::CWD,
            OsStr::new(if path.is_absolute() { "/" } else { "." }),
        )?;
        for component in path.components() {
            let name = match component {
                Component::RootDir | Component::CurDir => continue,
                Component::Normal(name) => name,
                Component::ParentDir => OsStr::new(".."),
                Component::Prefix(_) => return Err(invalid("unsupported destination prefix")),
            };
            fd = open_dir(&fd, name)?;
        }
        Ok((fd, name))
    }
    fn sync_dir(fd: impl AsFd) -> Result<()> {
        syscall("sync directory", fs::fsync(fd))
    }
    fn write_file(parent: impl AsFd, name: &OsStr, payload: &[u8], executable: bool) -> Result<()> {
        let fd = syscall(
            "create exclusive file",
            fs::openat(
                parent,
                name,
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            ),
        )?;
        let mut file = File::from(fd);
        io("write file", file.write_all(payload))?;
        let mode = if executable { 0o755 } else { 0o644 };
        syscall(
            "set file executable mode",
            fs::fchmod(&file, Mode::from_raw_mode(mode)),
        )?;
        io("sync file", file.sync_all())?;
        // F_FULLFSYNC is checked on regular files, not directories (which use
        // fsync). This matches the store's macOS flush policy, not a custody claim.
        #[cfg(target_os = "macos")]
        syscall("full sync file", fs::fcntl_fullfsync(&file))?;
        Ok(())
    }

    pub(super) fn restore(
        engine: &Engine,
        root: Id,
        destination: &Path,
        limits: Limits,
    ) -> Result<()> {
        preflight(engine, root, limits)?;
        let (parent, name) = parent(destination)?;
        let root_fd = new_dir(&parent, name)?;
        sync_dir(&parent)?;
        let mut directories = vec![root_fd];
        let mut walk = Walk::new(engine, root, limits)?;
        while let Some(event) = walk.next()? {
            match event {
                Event::Leave => {
                    let fd = directories
                        .pop()
                        .ok_or_else(|| invalid("unbalanced restore traversal"))?;
                    sync_dir(&fd)?;
                    if let Some(parent) = directories.last() {
                        sync_dir(parent)?;
                    }
                }
                Event::Entry(name, entry) => {
                    let parent = directories
                        .last()
                        .ok_or_else(|| invalid("missing restore directory"))?;
                    let name = OsStr::from_bytes(&name);
                    match entry {
                        Entry::Directory(_) => {
                            let child = new_dir(parent, name)?;
                            sync_dir(parent)?;
                            directories.push(child);
                        }
                        Entry::File {
                            blob: id,
                            executable,
                        } => {
                            write_file(parent, name, blob(engine, *id)?, *executable)?;
                            sync_dir(parent)?;
                        }
                        Entry::Symlink(id) => {
                            syscall(
                                "create exclusive symlink",
                                fs::symlinkat(OsStr::from_bytes(blob(engine, *id)?), parent, name),
                            )?;
                            sync_dir(parent)?;
                        }
                        _ => {
                            return Err(invalid("unmaterializable source binding after preflight"));
                        }
                    }
                }
            }
        }
        sync_dir(&parent)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::unix::fs::symlink;

        #[test]
        fn anchored_child_survives_name_replacement_without_following_escape() {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().canonicalize().unwrap();
            let parent = open_dir(fs::CWD, path.as_os_str()).unwrap();
            let root = new_dir(&parent, OsStr::new("fresh")).unwrap();
            let child = new_dir(&root, OsStr::new("child")).unwrap();
            std::fs::create_dir(path.join("outside")).unwrap();
            std::fs::write(path.join("outside/sentinel"), b"untouched").unwrap();
            std::fs::rename(path.join("fresh/child"), path.join("fresh/original")).unwrap();
            symlink(path.join("outside"), path.join("fresh/child")).unwrap();
            write_file(&child, OsStr::new("sentinel"), b"source", false).unwrap();
            new_dir(&child, OsStr::new("grandchild")).unwrap();
            fs::symlinkat("../sentinel", &child, "source-link").unwrap();
            assert!(path.join("fresh/original/grandchild").is_dir());
            assert_eq!(
                std::fs::read_link(path.join("fresh/original/source-link")).unwrap(),
                Path::new("../sentinel")
            );
            assert!(!path.join("outside/grandchild").exists());
            assert!(!path.join("outside/source-link").is_symlink());
            assert!(open_dir(&root, OsStr::new("child")).is_err());
            assert_eq!(
                std::fs::read(path.join("outside/sentinel")).unwrap(),
                b"untouched"
            );
            assert_eq!(
                std::fs::read(path.join("fresh/original/sentinel")).unwrap(),
                b"source"
            );
            // A planted dangling leaf symlink cannot redirect a create either.
            symlink(path.join("outside/new"), path.join("fresh/original/link")).unwrap();
            assert!(write_file(&child, OsStr::new("link"), b"bad", false).is_err());
            assert!(new_dir(&child, OsStr::new("link")).is_err());
            assert!(fs::symlinkat("replacement", &child, "link").is_err());
            assert_eq!(
                std::fs::read_link(path.join("fresh/original/link")).unwrap(),
                path.join("outside/new")
            );
            assert!(!path.join("outside/new").exists());
        }
    }
}
