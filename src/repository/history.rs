//! Authored revision metadata. Publication and receipts remain owned by the
//! repository's single SQLite transaction; source edits never author implicitly.
use super::*;
use crate::Revision;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(test)]
#[path = "selection_tests.rs"]
mod selection_tests;

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
    ) -> Result<SelectionReceipt> {
        self.author(author, message, expected_generation, request_key, false)
    }

    /// Replace the selected tip with a new immutable revision of the same change.
    /// Authors selected source, retaining the predecessor's ChangeId and parents;
    /// it never parents the predecessor or reads live filesystem files.
    pub fn amend(
        &mut self,
        author: &str,
        message: &str,
        expected_generation: u64,
        request_key: &str,
    ) -> Result<SelectionReceipt> {
        self.author(author, message, expected_generation, request_key, true)
    }

    fn author(
        &mut self,
        author: &str,
        message: &str,
        expected_generation: u64,
        request_key: &str,
        amend: bool,
    ) -> Result<SelectionReceipt> {
        self.ensure_idle()?;
        check_request_key(request_key)?;
        let mutation = Mutation::Author {
            author,
            message,
            amend,
        };
        if let Some(prior) = self.selection_receipt(request_key)? {
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
            if let Some(prior) = self.selection_receipt(request_key)? {
                return self.reconcile(prior, expected_generation, mutation);
            }
            return Err(Error::Stale {
                expected: expected_generation,
                actual: current.generation,
            });
        }
        if amend {
            let head = current
                .revision
                .ok_or_else(|| invalid("amend requires an authored head"))?;
            if self.git_origin(head)?.is_some() {
                return Err(invalid(
                    "cannot amend a foreign import anchor; author a native child first",
                ));
            }
        }
        // Validate the source being authored, not every ancestor's source again.
        // Retained parents are immutable and were admitted when first authored.
        self.load_root(current.root)?;
        self.publish(expected_generation, request_key, Vec::new(), mutation)
    }

    /// Select an exact stored revision's source and head without touching files
    /// or named refs. Refuses selected-but-uncommitted source; an unborn selection
    /// must be the canonical empty Map. Exact retries precede all health checks.
    pub fn select_revision(
        &mut self,
        revision: Id,
        expected_generation: u64,
        request_key: &str,
    ) -> Result<SelectionReceipt> {
        self.ensure_idle()?;
        check_request_key(request_key)?;
        let mutation = Mutation::Select { revision };
        if let Some(prior) = self.selection_receipt(request_key)? {
            return self.reconcile(prior, expected_generation, mutation);
        }
        let current = self.status()?;
        if current.generation != expected_generation {
            if let Some(prior) = self.selection_receipt(request_key)? {
                return self.reconcile(prior, expected_generation, mutation);
            }
            return Err(Error::Stale {
                expected: expected_generation,
                actual: current.generation,
            });
        }
        require_clean_selection(&self.connection, current)?;
        let target = self.revision(revision)?;
        // Verify the destination's current source, not its accumulated history
        // or the departing source. Metadata retention is not a health certificate.
        self.load_root(target.root)?;
        self.publish(expected_generation, request_key, Vec::new(), mutation)
    }

    /// Restore the exact current operation's before-source/head values, under a
    /// NEW generation and receipt. No object deletion, filesystem or remote effect.
    /// Refuses if that operation no longer owns the current selection/generation.
    pub fn undo(
        &mut self,
        target: &str,
        expected_generation: u64,
        request_key: &str,
    ) -> Result<SelectionReceipt> {
        self.ensure_idle()?;
        check_request_key(request_key)?;
        let mutation = Mutation::Undo { target };
        if let Some(prior) = self.selection_receipt(request_key)? {
            return self.reconcile(prior, expected_generation, mutation);
        }
        check_request_key(target)?;
        let current = self.status()?;
        if current.generation != expected_generation {
            if let Some(prior) = self.selection_receipt(request_key)? {
                return self.reconcile(prior, expected_generation, mutation);
            }
            return Err(Error::Stale {
                expected: expected_generation,
                actual: current.generation,
            });
        }
        let restored = undo_selection(&self.connection, current, target)?;
        // Undo is new protection, not a promise to select a corrupt departed source.
        self.load_root(restored.root)?;
        if let Some(head) = restored.revision {
            self.revision(head)?;
        }
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

pub(super) fn require_clean_selection(connection: &Connection, current: Selection) -> Result<()> {
    let clean_root = match current.revision {
        Some(head) => read_revision(connection, head)?.root,
        None => Object::Map(crate::Node::Leaf(Vec::new())).id()?,
    };
    if current.root != clean_root {
        return Err(invalid(
            "revision selection refuses selected-but-uncommitted source; commit it or undo its current source operation first",
        ));
    }
    Ok(())
}

pub(super) fn undo_selection(
    connection: &Connection,
    current: Selection,
    target: &str,
) -> Result<Selection> {
    let original = match receipt(connection, target)? {
        Some(Receipt::Selection(original)) => original,
        Some(Receipt::Ref(_)) => {
            return Err(invalid(
                "selection undo cannot target a reference operation",
            ));
        }
        None => return Err(invalid("undo target has no committed receipt")),
    };
    if original.after.generation != current.generation {
        return Err(Error::Stale {
            expected: original.after.generation,
            actual: current.generation,
        });
    }
    if original.after != current {
        return Err(invalid(
            "undo target does not match the current source/head",
        ));
    }
    Ok(original.before)
}

pub(super) fn prepare(
    connection: &Connection,
    before: Selection,
    author: &str,
    message: &str,
    amend: bool,
) -> Result<(Id, Vec<u8>)> {
    let (parents, change, previous_time) = if amend {
        let id = before
            .revision
            .ok_or_else(|| invalid("amend requires an authored head"))?;
        let previous = read_revision(connection, id)?;
        (previous.parents, previous.change, Some(previous.time_ns))
    } else {
        let mut change = [0; 16];
        getrandom::fill(&mut change).map_err(std::io::Error::other)?;
        (before.revision.into_iter().collect(), change, None)
    };
    // An amend's actual parents are its predecessor's parents, not the
    // predecessor itself. Check those direct metadata rows before publication.
    for &parent in &parents {
        read_revision(connection, parent)?;
    }
    let mut time_ns: i64 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(std::io::Error::other)?
        .as_nanos()
        .try_into()
        .map_err(|_| Error::Limit("revision timestamp"))?;
    // Even an unchanged amend in the same clock tick is a distinct immutable
    // revision. No monotonic-clock claim is made for ordinary authored times.
    if previous_time == Some(time_ns) {
        time_ns = time_ns
            .checked_add(1)
            .ok_or(Error::Limit("revision timestamp"))?;
    }
    let object = Object::Revision(Revision {
        root: before.root,
        parents,
        change,
        author: author.to_owned(),
        message: message.to_owned(),
        time_ns,
        timezone_minutes: 0,
    });
    let payload = encode_bounded(&object)?;
    Ok((object.id()?, payload))
}
