use super::*;
use crate::codec::Conflict;
use crate::text;

#[derive(Clone, Copy, Debug)]
pub struct MergeBudget {
    pub paths: usize,
    pub text: text::Budget,
}
impl Default for MergeBudget {
    fn default() -> Self {
        Self {
            paths: 1_000_000,
            text: text::Budget::default(),
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Merged {
    pub root: Id,
    pub unresolved: bool,
}

/// Exact, in-memory operation witnesses. This is not a canonical patch object
/// or a durable retention receipt; a caller must protect its roots separately.
#[derive(Clone, Debug)]
pub struct Candidate {
    base: Id,
    root: Id,
    relocations: Vec<Relocate>,
}
impl Candidate {
    pub fn base(&self) -> Id {
        self.base
    }
    pub fn root(&self) -> Id {
        self.root
    }
}

impl Engine {
    pub fn prepare(
        &mut self,
        base: Id,
        relocations: &[Relocate],
        edits: &[Edit],
    ) -> Result<Candidate> {
        let root = self.transform(base, relocations, edits)?;
        Ok(Candidate {
            base,
            root,
            relocations: relocations.to_vec(),
        })
    }

    /// Compose exact witnessed candidates. An interacting recorded relocation
    /// conservatively preserves whole-root alternatives instead of guessing a
    /// remap or independently resolving correlated rename/delete bindings.
    pub fn compose(
        &mut self,
        left: &Candidate,
        right: &Candidate,
        budget: MergeBudget,
    ) -> Result<Merged> {
        if left.base != right.base {
            return Err(Error::Stale);
        }
        self.root(left.base)?;
        self.root(left.root)?;
        self.root(right.root)?;
        if left.root == right.root || left.root == left.base || right.root == right.base {
            return self.merge(left.base, left.root, right.root, budget);
        }
        for (moved, other) in [(left, right), (right, left)] {
            if moved.relocations.is_empty() {
                continue;
            }
            let changes = self.diff_with_limit(other.base, other.root, budget.paths)?;
            if moved.relocations.iter().any(|relocation| {
                changes.iter().any(|change| {
                    overlaps(&relocation.source, &change.path)
                        || overlaps(&relocation.destination, &change.path)
                })
            }) {
                if budget.paths == 0 {
                    return Err(Error::Limit("composition paths"));
                }
                let Entry::Conflict(root) = self.signed_value(
                    &[
                        (root_entry(left.root)?, 1),
                        (root_entry(right.root)?, 1),
                        (root_entry(left.base)?, -1),
                    ],
                    true,
                )?
                else {
                    return Err(invalid("expected unresolved relocation composition"));
                };
                return Ok(Merged {
                    root,
                    unresolved: true,
                });
            }
        }
        self.merge(left.base, left.root, right.root, budget)
    }

    /// Pure source composition. Limit/missing-object failures return no root;
    /// callers must separately retain input roots in their durability domain.
    pub fn merge(
        &mut self,
        base: Id,
        left: Id,
        right: Id,
        mut budget: MergeBudget,
    ) -> Result<Merged> {
        self.root(base)?;
        self.root(left)?;
        self.root(right)?;
        let value = self.merge_value(
            root_entry(base)?,
            root_entry(left)?,
            root_entry(right)?,
            true,
            &mut budget,
        )?;
        let root = match value {
            Entry::Directory(id) | Entry::Conflict(id) => id,
            _ => return Err(invalid("merge produced non-source root")),
        };
        Ok(Merged {
            root,
            unresolved: self.root(root)?.unresolved,
        })
    }

    fn merge_value(
        &mut self,
        base: Entry,
        left: Entry,
        right: Entry,
        root: bool,
        budget: &mut MergeBudget,
    ) -> Result<Entry> {
        if left == right {
            return Ok(left);
        }
        if left == base {
            return Ok(right);
        }
        if right == base {
            return Ok(left);
        }
        if let Entry::Conflict(id) = base {
            return Err(Error::UnresolvedBase(id));
        }
        if budget.paths == 0 {
            return Err(Error::Limit("merge paths"));
        }
        budget.paths -= 1;
        if let (Entry::Directory(left_id), Entry::Directory(right_id)) = (&left, &right)
            && matches!(base, Entry::Directory(_) | Entry::Absent)
        {
            let base_id = if let Entry::Directory(id) = base {
                id
            } else {
                self.empty()?
            };
            let mut changed = Vec::new();
            self.diff_views(
                View::Node(base_id, 0),
                View::Node(*left_id, 0),
                &mut Vec::new(),
                &mut changed,
                budget.paths,
            )?;
            self.diff_views(
                View::Node(base_id, 0),
                View::Node(*right_id, 0),
                &mut Vec::new(),
                &mut changed,
                budget.paths,
            )?;
            let keys: BTreeSet<_> = changed.into_iter().map(|(key, _, _)| key).collect();
            if keys.len() > budget.paths {
                return Err(Error::Limit("merge changed paths"));
            }
            budget.paths -= keys.len();
            let mut merged = base_id;
            for key in keys {
                let value = self.merge_value(
                    self.lookup(base_id, &key)?,
                    self.lookup(*left_id, &key)?,
                    self.lookup(*right_id, &key)?,
                    false,
                    budget,
                )?;
                merged = self.update_path(merged, &[bytes_from_nibbles(&key)?], value)?;
            }
            return Ok(Entry::Directory(merged));
        }
        if let (
            Entry::File {
                blob: b,
                executable: bm,
            },
            Entry::File {
                blob: l,
                executable: lm,
            },
            Entry::File {
                blob: r,
                executable: rm,
            },
        ) = (&base, &left, &right)
        {
            let executable = if lm == rm || rm == bm { *lm } else { *rm };
            let blob = if l == r || r == b {
                Some(*l)
            } else if l == b {
                Some(*r)
            } else {
                let Object::Blob(b) = self.object(*b)? else {
                    return Err(invalid("file blob kind"));
                };
                let Object::Blob(l) = self.object(*l)? else {
                    return Err(invalid("file blob kind"));
                };
                let Object::Blob(r) = self.object(*r)? else {
                    return Err(invalid("file blob kind"));
                };
                match text::merge(b, l, r, budget.text) {
                    text::Outcome::Clean(bytes) => Some(self.blob(bytes)?),
                    text::Outcome::Conflict => None,
                    text::Outcome::Pending(reason) => return Err(Error::Limit(reason)),
                }
            };
            if let Some(blob) = blob {
                return Ok(Entry::File { blob, executable });
            }
            // The boolean mode is already resolved. Preserve exact content
            // alternatives with that same resolved mode on every signed term.
            return self.signed_value(
                &[
                    (
                        Entry::File {
                            blob: *l,
                            executable,
                        },
                        1,
                    ),
                    (
                        Entry::File {
                            blob: *r,
                            executable,
                        },
                        1,
                    ),
                    (
                        Entry::File {
                            blob: *b,
                            executable,
                        },
                        -1,
                    ),
                ],
                root,
            );
        }
        self.signed_value(&[(left, 1), (right, 1), (base, -1)], root)
    }

    /// Normalize only exact signed terms. This is not the lossy same-change
    /// heuristic, nor permission to infer an original recipe from an empty diff.
    fn signed_value(&mut self, inputs: &[(Entry, i16)], root: bool) -> Result<Entry> {
        let mut terms: BTreeMap<Vec<u8>, (Entry, i16)> = BTreeMap::new();
        for (input, sign) in inputs {
            if let Entry::Conflict(id) = input {
                let Object::Conflict(conflict) = self.object(*id)? else {
                    return Err(invalid("conflict dependency"));
                };
                for (entries, multiplier) in
                    [(&conflict.positive, *sign), (&conflict.negative, -*sign)]
                {
                    for (entry, count) in entries {
                        add(&mut terms, entry, multiplier * i16::from(*count))?;
                    }
                }
            } else {
                add(&mut terms, input, *sign)?;
            }
        }
        let mut positive = Vec::new();
        let mut negative = Vec::new();
        let mut total = 0u32;
        for (_, (entry, count)) in terms {
            if count == 0 {
                continue;
            }
            total += u32::from(count.unsigned_abs());
            if total > 64 {
                return Err(Error::Limit("signed conflict terms"));
            }
            let count_abs = u8::try_from(count.unsigned_abs())
                .map_err(|_| Error::Limit("signed multiplicity"))?;
            if count > 0 {
                positive.push((entry, count_abs));
            } else {
                negative.push((entry, count_abs));
            }
        }
        if positive.len() == 1 && positive[0].1 == 1 && negative.is_empty() {
            return Ok(positive.remove(0).0);
        }
        let mut bounds = Bounds {
            unresolved: true,
            ..Bounds::default()
        };
        for (entry, _) in positive.iter().chain(&negative) {
            let child = self.check_entry(entry, &[])?;
            bounds.depth = bounds.depth.max(child.depth);
            bounds.bytes = bounds.bytes.max(child.bytes);
        }
        let id = self.store(Object::Conflict(Conflict {
            region: u8::from(root),
            positive,
            negative,
        }))?;
        self.roots.insert(id, bounds);
        Ok(Entry::Conflict(id))
    }
}
fn add(terms: &mut BTreeMap<Vec<u8>, (Entry, i16)>, entry: &Entry, count: i16) -> Result<()> {
    let item = terms
        .entry(entry.encode()?)
        .or_insert_with(|| (entry.clone(), 0));
    item.1 = item
        .1
        .checked_add(count)
        .ok_or(Error::Limit("signed multiplicity"))?;
    Ok(())
}
