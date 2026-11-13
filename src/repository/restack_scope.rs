//! Accepted local rewrite lineage, not a canonical object or transfer record.
use super::*;

const MAGIC: &[u8; 8] = b"AVCRST01";
const DOMAIN: &[u8] = b"AVC-RESTACK-SCOPE\0v1\0";
const FIXED_BYTES: usize = 110;
pub(super) const MAX_SCOPE_BYTES: usize = FIXED_BYTES + 66 * MAX_OBJECTS;

/// One exact predecessor/successor pair in oldest-first replay order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RevisionRewrite {
    pub before: Id,
    pub after: Id,
}

pub(super) struct RestackScope {
    base: Id,
    onto: Id,
    rewrites: Vec<RevisionRewrite>,
    bytes: Vec<u8>,
}
impl RestackScope {
    pub(super) fn new(base: Id, onto: Id, rewrites: Vec<RevisionRewrite>) -> Result<Self> {
        validate(base, onto, &rewrites)?;
        let mut bytes = Vec::with_capacity(FIXED_BYTES + 66 * rewrites.len());
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&base.to_bytes()?);
        bytes.extend_from_slice(&onto.to_bytes()?);
        bytes.extend_from_slice(&(rewrites.len() as u32).to_le_bytes());
        for pair in &rewrites {
            bytes.extend_from_slice(&pair.before.to_bytes()?);
            bytes.extend_from_slice(&pair.after.to_bytes()?);
        }
        let checksum = checksum(&bytes);
        bytes.extend_from_slice(&checksum);
        Ok(Self {
            base,
            onto,
            rewrites,
            bytes,
        })
    }

    pub(super) fn decode(bytes: Vec<u8>) -> Result<Self> {
        if bytes.len() < FIXED_BYTES + 66 || bytes.len() > MAX_SCOPE_BYTES || &bytes[..8] != MAGIC {
            return Err(invalid("restack scope framing"));
        }
        let count = u32::from_le_bytes(
            bytes[74..78]
                .try_into()
                .map_err(|_| invalid("restack count"))?,
        ) as usize;
        if count == 0 || count > MAX_OBJECTS || bytes.len() != FIXED_BYTES + 66 * count {
            return Err(invalid("restack scope count or length"));
        }
        let end = bytes.len() - 32;
        if checksum(&bytes[..end]) != bytes[end..] {
            return Err(invalid("restack scope checksum"));
        }
        let base = Id::from_bytes(&bytes[8..41])?;
        let onto = Id::from_bytes(&bytes[41..74])?;
        let rewrites = bytes[78..end]
            .as_chunks::<66>()
            .0
            .iter()
            .map(|pair| {
                Ok(RevisionRewrite {
                    before: Id::from_bytes(&pair[..33])?,
                    after: Id::from_bytes(&pair[33..])?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        validate(base, onto, &rewrites)?;
        Ok(Self {
            base,
            onto,
            rewrites,
            bytes,
        })
    }

    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub(super) fn base(&self) -> Id {
        self.base
    }
    pub(super) fn onto(&self) -> Id {
        self.onto
    }
    pub(super) fn operation(&self) -> Operation {
        Operation::Restack {
            base: self.base,
            onto: self.onto,
            rewrites: self.rewrites.clone(),
        }
    }
    pub(super) fn check_endpoints(&self, before: Selection, after: Selection) -> Result<()> {
        let last = self
            .rewrites
            .last()
            .ok_or_else(|| invalid("empty restack scope"))?;
        if before.revision != Some(last.before) || after.revision != Some(last.after) {
            return Err(invalid("restack receipt endpoints"));
        }
        Ok(())
    }
}

fn checksum(bytes: &[u8]) -> [u8; 32] {
    let mut hash = blake3::Hasher::new();
    hash.update(DOMAIN);
    hash.update(bytes);
    *hash.finalize().as_bytes()
}
fn validate(base: Id, onto: Id, rewrites: &[RevisionRewrite]) -> Result<()> {
    if rewrites.is_empty() || rewrites.len() > MAX_OBJECTS {
        return Err(invalid("restack scope count"));
    }
    let mut before = BTreeSet::new();
    let mut after = BTreeSet::new();
    for id in [base, onto]
        .into_iter()
        .chain(rewrites.iter().flat_map(|p| [p.before, p.after]))
    {
        if id.kind != crate::REVISION_KIND {
            return Err(invalid("restack scope revision kind"));
        }
    }
    for pair in rewrites {
        if !before.insert(pair.before) || !after.insert(pair.after) {
            return Err(invalid("restack scope duplicate revision"));
        }
    }
    if before.contains(&base) || before.contains(&onto) {
        return Err(invalid("restack scope includes its base or target"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id(n: u32) -> Id {
        let mut digest = [0; 32];
        digest[..4].copy_from_slice(&n.to_le_bytes());
        Id {
            kind: crate::REVISION_KIND,
            digest,
        }
    }
    fn resign(bytes: &mut [u8]) {
        let end = bytes.len() - 32;
        let hash = checksum(&bytes[..end]);
        bytes[end..].copy_from_slice(&hash);
    }
    #[test]
    fn scope_codec_rejects_corruption_noncanonical_shapes_and_false_lineage() {
        let original = RestackScope::new(
            id(0),
            id(1),
            vec![
                RevisionRewrite {
                    before: id(2),
                    after: id(2),
                },
                RevisionRewrite {
                    before: id(3),
                    after: id(4),
                },
            ],
        )
        .unwrap();
        assert_eq!(
            RestackScope::decode(original.bytes.clone())
                .unwrap()
                .operation(),
            original.operation()
        );
        // Real no-ops and unchanged prefixes overlap the old/new ID sets.
        assert!(
            RestackScope::new(
                id(0),
                id(0),
                vec![RevisionRewrite {
                    before: id(2),
                    after: id(2)
                }]
            )
            .is_ok()
        );
        for length in [0, 7, 77, original.bytes.len() - 1, original.bytes.len() + 1] {
            let mut bytes = original.bytes.clone();
            bytes.resize(length, 0);
            assert!(RestackScope::decode(bytes).is_err());
        }
        for offset in [0, 8, 42, 78, original.bytes.len() - 1] {
            let mut bytes = original.bytes.clone();
            bytes[offset] ^= 1;
            assert!(RestackScope::decode(bytes).is_err());
        }
        for count in [0, 1, MAX_OBJECTS as u32 + 1, u32::MAX] {
            let mut bytes = original.bytes.clone();
            bytes[74..78].copy_from_slice(&count.to_le_bytes());
            resign(&mut bytes);
            assert!(RestackScope::decode(bytes).is_err());
        }
        // A valid checksum does not excuse a wrong kind or duplicate pair ID.
        for (offset, replacement) in [
            (8, vec![crate::BLOB_KIND]),
            (144, id(2).to_bytes().unwrap().to_vec()),
            (177, id(2).to_bytes().unwrap().to_vec()),
        ] {
            let mut bytes = original.bytes.clone();
            bytes[offset..offset + replacement.len()].copy_from_slice(&replacement);
            resign(&mut bytes);
            assert!(RestackScope::decode(bytes).is_err());
        }
        assert!(RestackScope::new(id(0), id(1), Vec::new()).is_err());
        for before in [id(0), id(1)] {
            assert!(
                RestackScope::new(
                    id(0),
                    id(1),
                    vec![RevisionRewrite {
                        before,
                        after: id(2)
                    }]
                )
                .is_err()
            );
        }
    }
    #[test]
    fn maximum_lineage_count_fits_one_bounded_private_row_and_decodes_exactly() {
        let rewrites = (0..MAX_OBJECTS as u32)
            .map(|n| RevisionRewrite {
                before: id(n + 2),
                after: id(n + 2),
            })
            .collect();
        let scope = RestackScope::new(id(0), id(1), rewrites).unwrap();
        assert_eq!(scope.bytes.len(), MAX_SCOPE_BYTES);
        assert!(scope.bytes.len() + 1024 < MAX_OBJECT_BYTES);
        assert_eq!(
            RestackScope::decode(scope.bytes.clone())
                .unwrap()
                .operation(),
            scope.operation()
        );
    }
}
