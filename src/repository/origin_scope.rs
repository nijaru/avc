//! Complete accepted origin declarations, retained with the history-import receipt.
use super::*;
use crate::git::GitOid;

const MAGIC: &[u8; 8] = b"AVCGOR01";
const CLAIM_BYTES: usize = 33 + 21 + 32;
const HEADER_BYTES: usize = 12;
const DIGEST_BYTES: usize = 32;
pub(super) const MAX_SCOPE_BYTES: usize = HEADER_BYTES + MAX_OBJECTS * CLAIM_BYTES + DIGEST_BYTES;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct OriginScope(Vec<u8>);

impl OriginScope {
    pub(super) fn new(origins: &[GitOrigin]) -> Result<Self> {
        if origins.len() > MAX_OBJECTS {
            return Err(Error::Limit("history origin count"));
        }
        let mut sorted = BTreeMap::new();
        for origin in origins {
            if origin.revision.kind != crate::REVISION_KIND
                || sorted.insert(origin.revision, origin).is_some()
            {
                return Err(invalid("history origin scope revision"));
            }
        }
        let mut bytes =
            Vec::with_capacity(HEADER_BYTES + origins.len() * CLAIM_BYTES + DIGEST_BYTES);
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&(origins.len() as u32).to_le_bytes());
        for (revision, origin) in sorted {
            bytes.extend_from_slice(&revision.to_bytes()?);
            bytes.extend_from_slice(&origin.commit.to_bytes());
            bytes.extend_from_slice(blake3::hash(&origin.raw_commit).as_bytes());
        }
        let digest = digest(&bytes);
        bytes.extend_from_slice(&digest);
        Ok(Self(bytes))
    }

    pub(super) fn decode(bytes: Vec<u8>) -> Result<Self> {
        if bytes.len() < HEADER_BYTES + DIGEST_BYTES
            || bytes.len() > MAX_SCOPE_BYTES
            || &bytes[..8] != MAGIC
        {
            return Err(invalid("history origin scope framing"));
        }
        let count = u32::from_le_bytes(
            bytes[8..12]
                .try_into()
                .map_err(|_| invalid("origin count"))?,
        ) as usize;
        if count > MAX_OBJECTS || bytes.len() != HEADER_BYTES + count * CLAIM_BYTES + DIGEST_BYTES {
            return Err(invalid("history origin scope count"));
        }
        let end = bytes.len() - DIGEST_BYTES;
        if digest(&bytes[..end]).as_slice() != &bytes[end..] {
            return Err(invalid("history origin scope checksum"));
        }
        let mut previous = None;
        for claim in bytes[HEADER_BYTES..end].as_chunks::<CLAIM_BYTES>().0 {
            let revision = Id::from_bytes(&claim[..33])?;
            if revision.kind != crate::REVISION_KIND || previous.is_some_and(|id| id >= revision) {
                return Err(invalid("history origin scope ordering/kind"));
            }
            GitOid::from_bytes(&claim[33..54])?;
            previous = Some(revision);
        }
        Ok(Self(bytes))
    }

    pub(super) fn bytes(&self) -> &[u8] {
        &self.0
    }

    pub(super) fn claim(&self, revision: Id) -> Result<Option<(GitOid, &[u8])>> {
        let wanted = revision.to_bytes()?;
        let claims = &self.0[HEADER_BYTES..self.0.len() - DIGEST_BYTES];
        let mut low = 0;
        let mut high = claims.len() / CLAIM_BYTES;
        while low < high {
            let middle = low + (high - low) / 2;
            let claim = &claims[middle * CLAIM_BYTES..(middle + 1) * CLAIM_BYTES];
            match claim[..33].cmp(&wanted) {
                std::cmp::Ordering::Less => low = middle + 1,
                std::cmp::Ordering::Greater => high = middle,
                std::cmp::Ordering::Equal => {
                    return Ok(Some((GitOid::from_bytes(&claim[33..54])?, &claim[54..])));
                }
            }
        }
        Ok(None)
    }

    pub(super) fn load(connection: &Connection) -> Result<Option<Self>> {
        // A fresh-store history import can occur only once. The partial unique
        // index enforces that invariant and bounds this lookup independently of history.
        let row: Option<Option<Vec<u8>>> = connection
            .query_row(
                "SELECT import_origins FROM operations WHERE kind = 'import-history'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        row.map(|bytes| Self::decode(bytes.ok_or_else(|| invalid("missing history origin scope"))?))
            .transpose()
    }
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    let mut hash = blake3::Hasher::new();
    hash.update(b"AVC-HISTORY-ORIGINS\0v1\0");
    hash.update(bytes);
    *hash.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin(n: u8) -> GitOrigin {
        let mut oid = [n; 21];
        oid[0] = 1;
        GitOrigin {
            revision: Id {
                kind: crate::REVISION_KIND,
                digest: [n; 32],
            },
            commit: GitOid::from_bytes(&oid).unwrap(),
            tree: GitOid::from_bytes(&oid).unwrap(),
            parents: Vec::new(),
            raw_commit: vec![n; 7],
        }
    }
    fn resign(bytes: &mut [u8]) {
        let end = bytes.len() - DIGEST_BYTES;
        let checksum = digest(&bytes[..end]);
        bytes[end..].copy_from_slice(&checksum);
    }

    #[test]
    fn scope_binds_complete_sorted_claims_and_raw_commit_digests() {
        let first = origin(1);
        let last = origin(3);
        let scope = OriginScope::new(&[last.clone(), first.clone()]).unwrap();
        assert_eq!(OriginScope::decode(scope.bytes().to_vec()).unwrap(), scope);
        assert_eq!(
            scope.claim(first.revision).unwrap(),
            Some((
                first.commit,
                blake3::hash(&first.raw_commit).as_bytes().as_slice()
            ))
        );
        assert_eq!(
            scope.claim(last.revision).unwrap(),
            Some((
                last.commit,
                blake3::hash(&last.raw_commit).as_bytes().as_slice()
            ))
        );
        assert_eq!(scope.claim(origin(2).revision).unwrap(), None);
        assert!(OriginScope::new(&[first.clone(), first]).is_err());
        let empty = OriginScope::new(&[]).unwrap();
        assert_eq!(empty.bytes().len(), 44);
        assert_eq!(OriginScope::decode(empty.bytes().to_vec()).unwrap(), empty);
        assert_eq!(empty.claim(last.revision).unwrap(), None);
    }

    #[test]
    fn damaged_or_noncanonical_scope_never_yields_partial_custody() {
        let scope = OriginScope::new(&[origin(1), origin(2)]).unwrap();
        for index in [0, 8, 12, 45, 66, scope.bytes().len() - 1] {
            let mut bytes = scope.bytes().to_vec();
            bytes[index] ^= 1;
            assert!(OriginScope::decode(bytes).is_err());
        }
        for kind in [
            "order",
            "duplicate",
            "revision-kind",
            "algorithm",
            "count",
            "trailing",
        ] {
            let mut bytes = scope.bytes().to_vec();
            match kind {
                "order" => {
                    for i in 0..CLAIM_BYTES {
                        bytes.swap(HEADER_BYTES + i, HEADER_BYTES + CLAIM_BYTES + i);
                    }
                }
                "duplicate" => {
                    bytes.copy_within(
                        HEADER_BYTES..HEADER_BYTES + CLAIM_BYTES,
                        HEADER_BYTES + CLAIM_BYTES,
                    );
                }
                "revision-kind" => bytes[HEADER_BYTES] = crate::BLOB_KIND,
                "algorithm" => bytes[HEADER_BYTES + 33] = 2,
                "count" => bytes[8..12].copy_from_slice(&100_001_u32.to_le_bytes()),
                "trailing" => bytes.push(0),
                _ => unreachable!(),
            }
            resign(&mut bytes);
            assert!(OriginScope::decode(bytes).is_err(), "{kind}");
        }
    }
}
