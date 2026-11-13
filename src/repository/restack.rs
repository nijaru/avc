//! Atomic replay of an explicit nonempty linear descendant range.
use super::*;

impl Repository {
    /// Replay BASE's selected linear descendants onto ONTO, oldest first.
    /// BASE is excluded. Preserves every ChangeId and authored field, including
    /// empty changes. Does not discover other descendants, move refs or touch files.
    pub fn restack(
        &mut self,
        base: Id,
        onto: Id,
        expected_generation: u64,
        request_key: &str,
    ) -> Result<SelectionReceipt> {
        self.ensure_idle()?;
        check_request_key(request_key)?;
        if let Some(prior) = self.selection_receipt(request_key)? {
            return reconcile(prior, expected_generation, base, onto);
        }
        let current = self.status()?;
        if current.generation != expected_generation {
            if let Some(prior) = self.selection_receipt(request_key)? {
                return reconcile(prior, expected_generation, base, onto);
            }
            return Err(Error::Stale {
                expected: expected_generation,
                actual: current.generation,
            });
        }
        if base.kind != crate::REVISION_KIND || onto.kind != crate::REVISION_KIND {
            return Err(invalid("restack base and target must be revision IDs"));
        }
        let head = current
            .revision
            .ok_or_else(|| invalid("restack requires an authored head"))?;
        history::require_clean_selection(&self.connection, current)?;
        let input = collect_closure([head, onto], |id| self.read_object(id))?;
        let mut engine = Engine::new();
        engine.import_roots(
            input.iter().map(|(id, bytes)| (*id, bytes.clone())),
            [head, onto],
            limits(),
        )?;
        let mut range = Vec::new();
        let mut changes = BTreeSet::new();
        let mut cursor = head;
        // Stop at BASE before inspecting its parents: a merge or import anchor
        // is a valid boundary, but must never be flattened inside the range.
        while cursor != base {
            let revision = revision(&engine, cursor)?;
            let [parent] = revision.parents.as_slice() else {
                return Err(invalid(
                    "restack requires a linear range reaching its exact base",
                ));
            };
            if !changes.insert(revision.change) {
                return Err(invalid("restack range contains a repeated ChangeId"));
            }
            range.push((
                cursor,
                revision.clone(),
                self::revision(&engine, *parent)?.root,
            ));
            cursor = *parent;
        }
        if range.is_empty() {
            return Err(invalid("restack requires a nonempty descendant range"));
        }
        range.reverse();
        let mut pending = vec![onto];
        let mut seen = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            let target = revision(&engine, id)?;
            if changes.contains(&target.change) {
                return Err(invalid(
                    "restack target native ancestry already contains a range ChangeId",
                ));
            }
            pending.extend(target.parents.iter().copied());
        }
        let mut root = revision(&engine, onto)?.root;
        drop(engine);

        let mut output = ClosureBuilder::default();
        output.extend([onto], |id| {
            input.get(&id).cloned().ok_or(Error::MissingObject(id))
        })?;
        let mut tip = onto;
        let mut rewrites = Vec::with_capacity(range.len());
        for (before, original, comparison) in range {
            let roots = [comparison, root, original.root];
            // A fresh source-only engine prevents unreachable merge scratch from
            // accumulating across the range. This is not a peak-RAM quota: one
            // merge and verification may hold intermediates and multiple copies.
            let records = collect_closure(roots, |id| {
                input
                    .get(&id)
                    .or_else(|| output.records.get(&id))
                    .cloned()
                    .ok_or(Error::MissingObject(id))
            })?;
            let mut scratch = Engine::new();
            scratch.import_roots(records, roots, limits())?;
            let rewritten = rebase::replay(&mut scratch, original, comparison, tip, root)?;
            let next_root = rewritten.root;
            let object = Object::Revision(rewritten);
            let after = object.id()?;
            let payload = encode_bounded(&object)?;
            // Count all output ownership, including objects reused from INPUT.
            // Earlier rewritten parents are already admitted and aren't fetched.
            output.extend([after], |id| {
                if id == after {
                    Ok(payload.clone())
                } else {
                    encode_bounded(scratch.object(id)?)
                }
            })?;
            rewrites.push(RevisionRewrite { before, after });
            root = next_root;
            tip = after;
        }
        drop(input);
        let scope = RestackScope::new(base, onto, rewrites)?;
        let mut verified = Engine::new();
        verified.import_roots(
            output
                .records
                .iter()
                .map(|(id, bytes)| (*id, bytes.clone())),
            [tip],
            limits(),
        )?;
        drop(verified);
        let prepared = self.prepare_objects(output.records)?;
        self.publish(
            expected_generation,
            request_key,
            prepared,
            Mutation::Restack {
                root,
                revision: tip,
                scope: &scope,
            },
        )
    }
}

fn revision(engine: &Engine, id: Id) -> Result<&crate::Revision> {
    match engine.object(id)? {
        Object::Revision(revision) => Ok(revision),
        _ => Err(invalid("restack ancestry requires revisions")),
    }
}

pub(super) fn reconcile(
    prior: SelectionReceipt,
    expected: u64,
    base: Id,
    onto: Id,
) -> Result<SelectionReceipt> {
    let matches = matches!(&prior.operation, Operation::Restack { base: retained_base, onto: retained_onto, .. }
        if *retained_base == base && *retained_onto == onto);
    if prior.before.generation != expected || !matches {
        return Err(Error::RequestKeyReuse(prior.request_key));
    }
    Ok(prior)
}
