//! Replay one authored change. All preparation is outside the publication lock;
//! the existing selection CAS and typed receipt own its atomic visibility.
use super::*;

impl Repository {
    /// Replay the selected change onto an exact native revision. Requires clean
    /// selected source and at most one original parent. Preserves ChangeId and
    /// authored metadata; does not restack descendants or touch live files.
    pub fn rebase(
        &mut self,
        onto: Id,
        expected_generation: u64,
        request_key: &str,
    ) -> Result<SelectionReceipt> {
        self.ensure_idle()?;
        check_request_key(request_key)?;
        if let Some(prior) = self.selection_receipt(request_key)? {
            return self.reconcile_rebase(prior, expected_generation, onto);
        }
        let current = self.status()?;
        if current.generation != expected_generation {
            if let Some(prior) = self.selection_receipt(request_key)? {
                return self.reconcile_rebase(prior, expected_generation, onto);
            }
            return Err(Error::Stale {
                expected: expected_generation,
                actual: current.generation,
            });
        }
        if onto.kind != crate::REVISION_KIND {
            return Err(invalid("rebase target must be a revision ID"));
        }
        let head = current
            .revision
            .ok_or_else(|| invalid("rebase requires an authored head"))?;
        if self.git_origin(head)?.is_some() {
            return Err(invalid(
                "cannot rebase a foreign import anchor as a Git change; author a native child first",
            ));
        }
        let mut revision = self.revision(head)?;
        if current.root != revision.root {
            return Err(invalid("rebase refuses selected-but-uncommitted source"));
        }
        if revision.parents.len() > 1 {
            return Err(invalid(
                "single-change rebase requires at most one original parent",
            ));
        }
        // Initial rebase verifies both complete owning graphs within one shared
        // budget. This is deliberately not an incremental-history performance claim.
        let records = collect_closure([head, onto], |id| self.read_object(id))?;
        let mut engine = Engine::new();
        engine.import_roots(records, [head, onto], limits())?;
        let mut pending = vec![onto];
        let mut seen = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            let Object::Revision(parent) = engine.object(id)? else {
                return Err(invalid("rebase ancestry requires revisions"));
            };
            if parent.change == revision.change {
                return Err(invalid(
                    "rebase target ancestry already contains this ChangeId",
                ));
            }
            pending.extend(parent.parents.iter().copied());
        }
        let base = match revision.parents.first() {
            Some(id) => match engine.object(*id)? {
                Object::Revision(parent) => parent.root,
                _ => return Err(invalid("rebase comparison parent must be a revision")),
            },
            None => engine.empty()?,
        };
        let Object::Revision(target) = engine.object(onto)? else {
            return Err(invalid("rebase target must be a revision"));
        };
        let merged = engine.merge(
            base,
            target.root,
            revision.root,
            crate::engine::MergeBudget::default(),
        )?;
        revision.root = merged.root;
        revision.parents = vec![onto];
        let id = engine.store(Object::Revision(revision))?;
        let records = collect_closure([id], |id| encode_bounded(engine.object(id)?))?;
        engine.verify(id, limits())?;
        let prepared = self.prepare_objects(records)?;
        self.publish(
            expected_generation,
            request_key,
            prepared,
            Mutation::Rebase {
                onto,
                root: merged.root,
                revision: id,
            },
        )
    }

    pub(super) fn reconcile_rebase(
        &self,
        prior: SelectionReceipt,
        expected: u64,
        onto: Id,
    ) -> Result<SelectionReceipt> {
        if prior.before.generation != expected || prior.operation != (Operation::Rebase { onto }) {
            return Err(Error::RequestKeyReuse(prior.request_key));
        }
        let before = self.revision(
            prior
                .before
                .revision
                .ok_or_else(|| invalid("rebase receipt has no predecessor"))?,
        )?;
        let after = self.revision(
            prior
                .after
                .revision
                .ok_or_else(|| invalid("rebase receipt has no revision"))?,
        )?;
        if before.root != prior.before.root
            || after.root != prior.after.root
            || after.parents != [onto]
            || after.change != before.change
            || after.author != before.author
            || after.message != before.message
            || after.time_ns != before.time_ns
            || after.timezone_minutes != before.timezone_minutes
        {
            return Err(invalid("rebase receipt does not match its revisions"));
        }
        Ok(prior)
    }
}
