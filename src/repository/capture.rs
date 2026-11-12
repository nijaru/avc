//! Explicit leaf capture selects pinned bytes; it does not author or write live files.
use super::*;

/// Exact requested filesystem scope. No recursive scan, inferred deletion or
/// implicit inclusion of new files. Relative directory inputs become absolute
/// lexically; constructing a retry identity never opens the source directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureScope {
    directory: PathBuf,
    paths: Vec<Vec<Vec<u8>>>,
}
impl CaptureScope {
    pub fn new(directory: &Path, mut paths: Vec<Vec<Vec<u8>>>) -> Result<Self> {
        let directory = std::path::absolute(directory)?;
        if directory
            .components()
            .any(|p| matches!(p, std::path::Component::ParentDir))
        {
            return Err(invalid("capture directory must not contain '..'"));
        }
        if directory.as_os_str().as_encoded_bytes().contains(&0) {
            return Err(invalid("capture directory contains NUL"));
        }
        if directory.as_os_str().as_encoded_bytes().len() > 4096 {
            return Err(Error::Limit("capture directory path"));
        }
        if paths.is_empty() || paths.len() > 1024 {
            return Err(Error::Limit(
                "capture requires 1..=1024 explicit leaf paths",
            ));
        }
        let mut size = 0usize;
        for path in &paths {
            crate::validate_path(path)?;
            if path.is_empty() || path.iter().any(|p| p == b".avc" || p == b".git") {
                return Err(invalid("capture refuses root and administration paths"));
            }
            size += path.iter().map(|p| p.len() + 1).sum::<usize>();
        }
        if size > 65536 {
            return Err(Error::Limit("capture scope path bytes"));
        }
        paths.sort();
        if paths.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(invalid("duplicate capture path"));
        }
        Ok(Self { directory, paths })
    }
    pub fn directory(&self) -> &Path {
        &self.directory
    }
    pub fn paths(&self) -> &[Vec<Vec<u8>>] {
        &self.paths
    }

    pub(super) fn encode(&self) -> Vec<u8> {
        let mut out = b"AVCCAP01".to_vec();
        let root = self.directory.as_os_str().as_encoded_bytes();
        out.extend_from_slice(&(root.len() as u32).to_le_bytes());
        out.extend_from_slice(root);
        out.extend_from_slice(&(self.paths.len() as u32).to_le_bytes());
        for path in &self.paths {
            out.extend_from_slice(&(path.len() as u16).to_le_bytes());
            for part in path {
                out.extend_from_slice(&(part.len() as u16).to_le_bytes());
                out.extend_from_slice(part);
            }
        }
        out
    }
    #[cfg(unix)]
    pub(super) fn decode(bytes: &[u8]) -> Result<Self> {
        use std::os::unix::ffi::OsStringExt;
        fn take<'a>(input: &mut &'a [u8], len: usize) -> Result<&'a [u8]> {
            if input.len() < len {
                return Err(invalid("truncated capture scope"));
            }
            let (value, rest) = input.split_at(len);
            *input = rest;
            Ok(value)
        }
        fn number(input: &mut &[u8], width: usize) -> Result<usize> {
            let mut n = 0usize;
            for (i, b) in take(input, width)?.iter().enumerate() {
                n |= usize::from(*b) << (i * 8);
            }
            Ok(n)
        }
        if bytes.len() > 131072 {
            return Err(Error::Limit("stored capture scope"));
        }
        let mut input = bytes;
        if take(&mut input, 8)? != b"AVCCAP01" {
            return Err(invalid("capture scope version"));
        }
        let len = number(&mut input, 4)?;
        if len > 4096 {
            return Err(Error::Limit("capture directory path"));
        }
        let directory = PathBuf::from(std::ffi::OsString::from_vec(
            take(&mut input, len)?.to_vec(),
        ));
        if !directory.is_absolute() {
            return Err(invalid("stored capture directory is not absolute"));
        }
        let count = number(&mut input, 4)?;
        if count == 0 || count > 1024 {
            return Err(Error::Limit("stored capture paths"));
        }
        let mut paths = Vec::with_capacity(count);
        for _ in 0..count {
            let count = number(&mut input, 2)?;
            if count == 0 || count > 256 {
                return Err(invalid("stored capture path components"));
            }
            let mut path = Vec::with_capacity(count);
            for _ in 0..count {
                let len = number(&mut input, 2)?;
                if len > 255 {
                    return Err(invalid("stored capture basename"));
                }
                path.push(take(&mut input, len)?.to_vec());
            }
            paths.push(path);
        }
        let scope = Self::new(&directory, paths)?;
        if !input.is_empty() || scope.encode() != bytes {
            return Err(invalid("noncanonical capture scope"));
        }
        Ok(scope)
    }
    #[cfg(not(unix))]
    pub(super) fn decode(_: &[u8]) -> Result<Self> {
        Err(invalid("capture requires Unix"))
    }
}

impl Repository {
    /// Observe explicit existing leaves once, then atomically select their exact
    /// bytes under the current source CAS. Unselected source and authored head
    /// remain unchanged. A retry recovers the original observation without any
    /// source filesystem access, even after later edits or selection changes.
    pub fn capture(&mut self, scope: &CaptureScope, expected: u64, key: &str) -> Result<Receipt> {
        self.ensure_idle()?;
        check_request_key(key)?;
        if let Some(prior) = self.receipt(key)? {
            return self.reconcile_capture(prior, expected, scope);
        }
        let before = self.status()?;
        if before.generation != expected {
            if let Some(prior) = self.receipt(key)? {
                return self.reconcile_capture(prior, expected, scope);
            }
            return Err(Error::Stale {
                expected,
                actual: before.generation,
            });
        }
        let mut engine = self.load_root(before.root)?;
        // Resolve the explicitly supplied root only, after retry reconciliation.
        // Trusted roots such as macOS /var commonly contain symlink aliases.
        // The reader rejects physical admin roots and anchors every descendant.
        let directory = scope.directory().canonicalize()?;
        let observation =
            crate::capture::observe(&mut engine, before.root, &directory, scope.paths())?;
        if observation.witnesses.is_empty() || observation.witnesses.len() > 262144 {
            return Err(Error::Limit("capture observation witness bytes"));
        }
        let records = collect_closure([observation.root], |id| encode_bounded(engine.object(id)?))?;
        engine.verify(observation.root, limits())?;
        let prepared = self.prepare_objects(records)?;
        self.publish(
            expected,
            key,
            prepared,
            Mutation::Capture {
                root: observation.root,
                scope,
                witnesses: &observation.witnesses,
            },
        )
    }
    pub(super) fn reconcile_capture(
        &self,
        prior: Receipt,
        expected: u64,
        scope: &CaptureScope,
    ) -> Result<Receipt> {
        if prior.before.generation != expected
            || prior.operation
                != (Operation::Capture {
                    scope: scope.clone(),
                })
        {
            return Err(Error::RequestKeyReuse(prior.request_key));
        }
        Ok(prior)
    }
    /// Retained versioned observation evidence, not a filesystem snapshot proof.
    /// Unlike receipt recovery, this explicitly checks witness-record custody.
    pub fn capture_observation(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let Some(prior) = self.receipt(key)? else {
            return Ok(None);
        };
        let Operation::Capture { scope } = prior.operation else {
            return Ok(None);
        };
        let row: Option<(Vec<u8>, Vec<u8>)> = self
            .connection
            .query_row(
                "SELECT witnesses, checksum FROM capture_observations WHERE request_key = ?1",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (witnesses, checksum) = row.ok_or_else(|| invalid("missing capture observation"))?;
        if witnesses.is_empty()
            || witnesses.len() > 262144
            || checksum != observation_checksum(prior.after.root, &scope, &witnesses)?
        {
            return Err(invalid("capture observation checksum"));
        }
        Ok(Some(witnesses))
    }
}
fn observation_checksum(root: Id, scope: &CaptureScope, witnesses: &[u8]) -> Result<Vec<u8>> {
    let scope = scope.encode();
    let mut h = blake3::Hasher::new();
    h.update(b"AVC-CAPTURE-OBSERVATION\0v1\0");
    h.update(&root.to_bytes()?);
    h.update(&(scope.len() as u32).to_le_bytes());
    h.update(&scope);
    h.update(&(witnesses.len() as u32).to_le_bytes());
    h.update(witnesses);
    Ok(h.finalize().as_bytes().to_vec())
}
pub(super) fn insert_observation(
    connection: &Connection,
    key: &str,
    root: Id,
    scope: &CaptureScope,
    witnesses: &[u8],
) -> Result<()> {
    connection.execute(
        "INSERT INTO capture_observations VALUES (?1, ?2, ?3)",
        params![
            key,
            witnesses,
            observation_checksum(root, scope, witnesses)?
        ],
    )?;
    Ok(())
}
