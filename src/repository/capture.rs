//! Explicit capture selects pinned bytes; it does not author or write live files.
use super::*;
use crate::capture::{CaptureMode, MAX_WITNESS_BYTES};

/// Exact requested filesystem scope and inclusion mode. Recursive scopes include
/// new descendants, but neither mode infers deletions. Relative directory inputs
/// become absolute lexically; constructing retry identity never opens the source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureScope {
    directory: PathBuf,
    paths: Vec<Vec<Vec<u8>>>,
    mode: CaptureMode,
}
impl CaptureScope {
    /// Select explicit file or symlink leaves only.
    pub fn new(directory: &Path, paths: Vec<Vec<Vec<u8>>>) -> Result<Self> {
        Self::with_mode(directory, paths, CaptureMode::Leaves)
    }
    /// Include complete existing directory descendants. An empty path selects
    /// the root directory. Missing previously selected bindings still refuse.
    pub fn recursive(directory: &Path, paths: Vec<Vec<Vec<u8>>>) -> Result<Self> {
        Self::with_mode(directory, paths, CaptureMode::Recursive)
    }
    fn with_mode(
        directory: &Path,
        mut paths: Vec<Vec<Vec<u8>>>,
        mode: CaptureMode,
    ) -> Result<Self> {
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
        crate::capture::validate_paths(&paths, mode)?;
        paths.sort();
        Ok(Self {
            directory,
            paths,
            mode,
        })
    }
    pub fn directory(&self) -> &Path {
        &self.directory
    }
    pub fn paths(&self) -> &[Vec<Vec<u8>>] {
        &self.paths
    }

    pub fn mode(&self) -> CaptureMode {
        self.mode
    }

    pub(super) fn encode(&self) -> Vec<u8> {
        let mut out = b"AVCCAP02".to_vec();
        out.push(match self.mode {
            CaptureMode::Leaves => 0,
            CaptureMode::Recursive => 1,
        });
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
        if take(&mut input, 8)? != b"AVCCAP02" {
            return Err(invalid("capture scope version"));
        }
        let mode = match take(&mut input, 1)?[0] {
            0 => CaptureMode::Leaves,
            1 => CaptureMode::Recursive,
            _ => return Err(invalid("capture inclusion mode")),
        };
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
            if count > 256 {
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
        let scope = Self::with_mode(&directory, paths, mode)?;
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
    /// Observe explicit existing scopes once, then atomically select their exact
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
        let observation = crate::capture::observe(
            &mut engine,
            before.root,
            &directory,
            scope.paths(),
            scope.mode(),
        )?;
        if observation.witnesses.is_empty() || observation.witnesses.len() > MAX_WITNESS_BYTES {
            return Err(Error::Limit("capture observation witness bytes"));
        }
        crate::capture::report(&observation.witnesses)?;
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
            || witnesses.len() > MAX_WITNESS_BYTES
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_mode_and_root_round_trip_without_opening_the_directory() {
        let directory = Path::new("/nonexistent-avc-scope/input");
        let root = CaptureScope::recursive(directory, vec![Vec::new()]).unwrap();
        let encoded = root.encode();
        assert!(encoded.starts_with(b"AVCCAP02\x01"));
        assert_eq!(CaptureScope::decode(&encoded).unwrap(), root);
        assert!(CaptureScope::new(directory, vec![Vec::new()]).is_err());
        let leaf = CaptureScope::new(directory, vec![vec![b"a".to_vec()]]).unwrap();
        let recursive = CaptureScope::recursive(directory, vec![vec![b"a".to_vec()]]).unwrap();
        assert_ne!(leaf, recursive);
        assert_ne!(leaf.encode(), recursive.encode());
        assert_eq!(CaptureScope::decode(&leaf.encode()).unwrap(), leaf);
        assert_eq!(
            CaptureScope::decode(&recursive.encode()).unwrap(),
            recursive
        );
    }

    #[test]
    fn stored_scope_rejects_unknown_mode_old_version_and_trailing_or_noncanonical_data() {
        let root = CaptureScope::recursive(Path::new("/input"), vec![Vec::new()]).unwrap();
        for mode in [0, 2, 255] {
            let mut bytes = root.encode();
            bytes[8] = mode;
            assert!(CaptureScope::decode(&bytes).is_err());
        }
        let mut bytes = root.encode();
        bytes[7] = b'1';
        assert!(CaptureScope::decode(&bytes).is_err());
        let mut bytes = root.encode();
        bytes.push(0);
        assert!(CaptureScope::decode(&bytes).is_err());
        let mut bytes = CaptureScope::new(
            Path::new("/input"),
            vec![vec![b"a".to_vec()], vec![b"b".to_vec()]],
        )
        .unwrap()
        .encode();
        let n = bytes.len();
        bytes.swap(n - 1, n - 6);
        assert!(CaptureScope::decode(&bytes).is_err());
    }
}
