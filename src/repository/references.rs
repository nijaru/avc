//! Native revision references are independent chains in the immutable journal.
use super::*;
use std::str::FromStr;

/// Exact UTF-8 bytes; slash segments have no filesystem interpretation.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct RefName(String);

impl FromStr for RefName {
    type Err = Error;

    fn from_str(name: &str) -> Result<Self> {
        if name.is_empty()
            || name.len() > 1024
            || name.chars().any(|c| c.is_whitespace() || c.is_control())
            || name.split('/').any(|s| matches!(s, "" | "." | ".."))
        {
            return Err(invalid(
                "reference name must be 1..=1024 UTF-8 bytes, with nonempty slash segments other than . or .. and no whitespace or control characters",
            ));
        }
        Ok(Self(name.to_owned()))
    }
}

impl RefName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RefValue {
    pub revision: Id,
    pub generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NamedRef {
    pub name: RefName,
    pub value: RefValue,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RefReceipt {
    pub request_key: String,
    pub name: RefName,
    pub before: Option<RefValue>,
    pub after: RefValue,
}

// The birth index enumerates names, not historical transitions. The correlated
// maximum and final join both seek the full generation index in one snapshot.
const LIST: &str = "SELECT birth.ref_name, latest.after_ref_revision, latest.after_ref_generation
    FROM operations AS birth
    JOIN operations AS latest ON latest.ref_name = birth.ref_name
      AND latest.after_ref_generation = (SELECT MAX(tip.after_ref_generation) FROM operations AS tip WHERE tip.ref_name = birth.ref_name)
    WHERE birth.kind = 'ref-set' AND birth.before_ref_generation IS NULL AND birth.ref_name > ?1
    ORDER BY birth.ref_name LIMIT ?2";

impl Repository {
    /// Metadata only: a reference does not certify its target's payload health.
    pub fn reference(&self, name: &RefName) -> Result<Option<RefValue>> {
        self.ensure_idle()?;
        current(&self.connection, name)
    }

    /// A bounded, byte-ordered page from one SQL read snapshot.
    pub fn references(&self, after: Option<&RefName>, limit: usize) -> Result<Vec<NamedRef>> {
        self.ensure_idle()?;
        if limit > MAX_OBJECTS {
            return Err(Error::Limit("reference records"));
        }
        let mut statement = self.connection.prepare(LIST)?;
        let mut rows = statement.query(params![after.map_or("", RefName::as_str), limit as i64])?;
        let mut result = Vec::new();
        while let Some(row) = rows.next()? {
            let name: String = row.get(0)?;
            result.push(NamedRef {
                name: name.parse()?,
                value: decode_value(row.get(1)?, row.get(2)?)?,
            });
        }
        Ok(result)
    }

    /// CAS only this name. Exact retries recover the original transition before
    /// stale checks or target validation, without consulting the selection.
    pub fn set_reference(
        &mut self,
        name: &RefName,
        revision: Id,
        expected_generation: u64,
        request_key: &str,
    ) -> Result<RefReceipt> {
        self.ensure_idle()?;
        check_request_key(request_key)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(prior) = receipt(&tx, request_key)? {
            let recovered = match prior {
                Receipt::Ref(prior)
                    if prior.name == *name
                        && prior.after.revision == revision
                        && prior.before.map_or(0, |value| value.generation)
                            == expected_generation =>
                {
                    Ok(prior)
                }
                _ => Err(Error::RequestKeyReuse(request_key.to_owned())),
            };
            tx.rollback()?;
            let prior = recovered?;
            durability_fence(&self.connection)?;
            return Ok(prior);
        }
        let before = current(&tx, name)?;
        let actual = before.map_or(0, |value| value.generation);
        if actual != expected_generation {
            return Err(Error::StaleReference {
                name: name.clone(),
                expected: expected_generation,
                actual,
            });
        }
        let next = i64::try_from(expected_generation)
            .ok()
            .and_then(|generation| generation.checked_add(1))
            .ok_or(Error::Limit("reference generation exhausted"))?;
        // Checks the stored revision and presence of direct owning dependencies,
        // not the accumulated historical closure or any foreign-origin records.
        history::read_revision(&tx, revision)?;
        tx.execute(
            "INSERT INTO operations (request_key, kind, ref_name, before_ref_generation, after_ref_generation, after_ref_revision) VALUES (?1, 'ref-set', ?2, ?3, ?4, ?5)",
            params![request_key, name.as_str(), before.map(|value| value.generation as i64), next, revision.to_bytes()?.as_slice()],
        )?;
        tx.commit()?;
        Ok(RefReceipt {
            request_key: request_key.to_owned(),
            name: name.clone(),
            before,
            after: RefValue {
                revision,
                generation: next as u64,
            },
        })
    }
}

fn decode_value(revision: Vec<u8>, generation: i64) -> Result<RefValue> {
    let revision = Id::from_bytes(&revision)?;
    if revision.kind != crate::REVISION_KIND || generation <= 0 {
        return Err(invalid("reference value"));
    }
    Ok(RefValue {
        revision,
        generation: generation as u64,
    })
}

fn current(connection: &Connection, name: &RefName) -> Result<Option<RefValue>> {
    let row = connection.query_row(
        "SELECT after_ref_revision, after_ref_generation FROM operations WHERE ref_name = ?1 ORDER BY after_ref_generation DESC LIMIT 1",
        [name.as_str()], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?;
    row.map(|(revision, generation)| decode_value(revision, generation))
        .transpose()
}

pub(super) type StoredRefReceipt = (String, Option<i64>, i64, Vec<u8>);

pub(super) fn decode_receipt(
    connection: &Connection,
    key: &str,
    (name, before_generation, after_generation, after_revision): StoredRefReceipt,
) -> Result<RefReceipt> {
    let name: RefName = name.parse()?;
    let after = decode_value(after_revision, after_generation)?;
    if before_generation.unwrap_or(0).checked_add(1) != Some(after_generation) {
        return Err(invalid("reference receipt generation step"));
    }
    let before = before_generation.map(|generation| {
        let revision = connection.query_row(
            "SELECT after_ref_revision FROM operations WHERE ref_name = ?1 AND after_ref_generation = ?2",
            params![name.as_str(), generation], |row| row.get(0),
        ).optional()?.ok_or_else(|| invalid("missing reference receipt predecessor"))?;
        decode_value(revision, generation)
    }).transpose()?;
    Ok(RefReceipt {
        request_key: key.to_owned(),
        name,
        before,
        after,
    })
}

#[cfg(test)]
#[path = "reference_tests.rs"]
mod tests;
