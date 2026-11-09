//! Authored revision metadata. Publication and receipts remain owned by the
//! repository's single SQLite transaction; source edits never author implicitly.
use super::*;
use crate::Revision;
use std::time::{SystemTime, UNIX_EPOCH};

impl Repository {
    /// Author the currently selected source, parenting the last authored revision.
    /// An explicit commit may have unchanged or empty source. Random ChangeId and
    /// UTC timestamp are generated once, inside publication; retries recover them
    /// from the original immutable revision, even after later selection changes.
    pub fn commit(
        &mut self,
        author: &str,
        message: &str,
        expected_generation: u64,
        request_key: &str,
    ) -> Result<Receipt> {
        self.ensure_idle()?;
        check_request_key(request_key)?;
        let mutation = Mutation::Commit { author, message };
        if let Some(prior) = self.receipt(request_key)? {
            return self.reconcile(prior, expected_generation, mutation);
        }
        if author.trim().is_empty() {
            return Err(invalid("author must not be blank"));
        }
        if author.len() > crate::codec::MAX_AUTHOR || message.len() > crate::codec::MAX_MESSAGE {
            return Err(Error::Limit("revision metadata"));
        }
        let current = self.status()?;
        if current.generation != expected_generation {
            if let Some(prior) = self.receipt(request_key)? {
                return self.reconcile(prior, expected_generation, mutation);
            }
            return Err(Error::Stale {
                expected: expected_generation,
                actual: current.generation,
            });
        }
        // Validate the source being authored, not every ancestor's source again.
        // Retained parents are immutable and were admitted when first authored.
        self.load_root(current.root)?;
        self.publish(expected_generation, request_key, Vec::new(), mutation)
    }

    /// Read checked revision metadata, without loading all historical file bytes.
    pub fn revision(&self, id: Id) -> Result<Revision> {
        self.ensure_idle()?;
        read_revision(&self.connection, id)
    }

    /// Newest-first first-parent history, starting at an explicit revision or
    /// the selected authored head. Limit bounds records; the existing byte budget
    /// also bounds aggregate metadata. No partial result is returned on error.
    pub fn log(&self, start: Option<Id>, limit: usize) -> Result<Vec<(Id, Revision)>> {
        self.ensure_idle()?;
        if limit > MAX_OBJECTS {
            return Err(Error::Limit("history records"));
        }
        let mut next = match start {
            Some(id) => Some(id),
            None => self.status()?.revision,
        };
        if next.is_some_and(|id| id.kind != crate::REVISION_KIND) {
            return Err(invalid("history requires a revision ID"));
        }
        let mut seen = BTreeSet::new();
        let mut records = Vec::new();
        let mut total = 0;
        while let Some(id) = next {
            if records.len() == limit {
                break;
            }
            if !seen.insert(id) {
                return Err(invalid("revision ancestry cycle"));
            }
            let payload = self.read_object(id)?;
            budget(&mut total, payload.len())?;
            let Object::Revision(revision) = Object::decode(id, &payload)? else {
                return Err(invalid("history requires a revision ID"));
            };
            next = revision.parents.first().copied();
            records.push((id, revision));
        }
        Ok(records)
    }
}

pub(super) fn read_revision(connection: &Connection, id: Id) -> Result<Revision> {
    if id.kind != crate::REVISION_KIND {
        return Err(invalid("expected a revision ID"));
    }
    let payload = read_object_optional(connection, id)?.ok_or(Error::MissingObject(id))?;
    let Object::Revision(revision) = Object::decode(id, &payload)? else {
        return Err(invalid("expected a revision ID"));
    };
    Ok(revision)
}

pub(super) fn prepare(
    connection: &Connection,
    before: Selection,
    author: &str,
    message: &str,
) -> Result<(Id, Vec<u8>)> {
    if let Some(parent) = before.revision {
        read_revision(connection, parent)?;
    }
    let mut change = [0; 16];
    getrandom::fill(&mut change).map_err(std::io::Error::other)?;
    let time_ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(std::io::Error::other)?
        .as_nanos()
        .try_into()
        .map_err(|_| Error::Limit("revision timestamp"))?;
    let object = Object::Revision(Revision {
        root: before.root,
        parents: before.revision.into_iter().collect(),
        change,
        author: author.to_owned(),
        message: message.to_owned(),
        time_ns,
        timezone_minutes: 0,
    });
    let payload = encode_bounded(&object)?;
    Ok((object.id()?, payload))
}
