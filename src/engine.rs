//! In-memory canonical source authority. No filesystem or durable-custody claims.
use crate::codec::{Entry, Error, Id, Node, Object, Result};
use std::collections::{BTreeMap, BTreeSet};
mod merge;
pub use merge::{Candidate, MergeBudget, Merged};

pub type Path = Vec<Vec<u8>>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Edit {
    pub path: Path,
    pub expected: Entry,
    pub value: Entry,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Relocate {
    pub source: Path,
    pub destination: Path,
    pub expected_source: Entry,
    pub expected_destination: Entry,
    pub copy: bool,
    pub overwrite: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Difference {
    pub path: Path,
    pub before: Entry,
    pub after: Entry,
}

/// Exact bounds derived from validated source, not from caller-supplied headers.
#[derive(Clone, Copy, Default, Debug)]
struct Bounds {
    depth: usize,
    bytes: usize,
    unresolved: bool,
}

#[derive(Clone, Copy, Default, Debug)]
struct Metrics {
    depth: usize,
    nibbles: usize,
    unresolved: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub objects: usize,
    pub bindings: usize,
    pub bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            objects: 1_000_000,
            bindings: 10_000_000,
            bytes: 1024 * 1024 * 1024,
        }
    }
}

/// Only constructed or fully verified objects enter this authority. Immutable
/// objects prepared by a failed operation may remain; no root is published.
#[derive(Default)]
pub struct Engine {
    objects: BTreeMap<Id, Object>,
    counts: BTreeMap<Id, u32>,
    roots: BTreeMap<Id, Bounds>,
    metrics: BTreeMap<Id, Metrics>,
}
impl Engine {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn object(&self, id: Id) -> Result<&Object> {
        self.objects.get(&id).ok_or(Error::NeedObjects(id))
    }
    pub fn objects(&self) -> impl Iterator<Item = (Id, &Object)> {
        self.objects.iter().map(|(id, o)| (*id, o))
    }
    pub fn object_count(&self) -> usize {
        self.objects.len()
    }
    pub fn blob(&mut self, bytes: Vec<u8>) -> Result<Id> {
        self.store(Object::Blob(bytes))
    }
    pub fn empty(&mut self) -> Result<Id> {
        let id = self.store_node(Node::Leaf(Vec::new()))?;
        self.roots.insert(id, Bounds::default());
        Ok(id)
    }
    pub(crate) fn store(&mut self, object: Object) -> Result<Id> {
        let id = object.id()?;
        if let Some(previous) = self.objects.get(&id) {
            if previous != &object {
                return Err(invalid("object hash collision"));
            }
        } else {
            self.objects.insert(id, object);
        }
        Ok(id)
    }
    fn store_node(&mut self, node: Node) -> Result<Id> {
        let count = match &node {
            Node::Leaf(entries) => {
                u32::try_from(entries.len()).map_err(|_| invalid("map count overflow"))?
            }
            Node::Branch { count, .. } => *count,
        };
        let id = self.store(Object::Map(node))?;
        self.counts.insert(id, count);
        self.node_metrics(id)?;
        Ok(id)
    }
    fn node(&self, id: Id) -> Result<&Node> {
        match self.object(id)? {
            Object::Map(node) => Ok(node),
            _ => Err(invalid("expected directory map")),
        }
    }
    fn count(&self, id: Id) -> Result<u32> {
        self.counts
            .get(&id)
            .copied()
            .ok_or_else(|| invalid("unverified map count"))
    }
    fn root(&self, root: Id) -> Result<Bounds> {
        match self.object(root)? {
            Object::Map(_) => {}
            Object::Conflict(conflict) if conflict.region == 1 => {}
            _ => return Err(invalid("invalid source-root type/region")),
        }
        self.bounds(root)
    }
    fn bounds(&self, id: Id) -> Result<Bounds> {
        self.roots
            .get(&id)
            .copied()
            .ok_or_else(|| invalid("unverified source bounds"))
    }

    /// Import is atomic at the authority boundary: decoding, hash, complete
    /// closure, rooted map context and path bounds all precede visibility.
    pub fn import(
        &mut self,
        records: impl IntoIterator<Item = (Id, Vec<u8>)>,
        root: Id,
        limits: Limits,
    ) -> Result<()> {
        self.import_roots(records, [root], limits)
    }

    pub(crate) fn import_roots(
        &mut self,
        records: impl IntoIterator<Item = (Id, Vec<u8>)>,
        roots: impl IntoIterator<Item = Id>,
        limits: Limits,
    ) -> Result<()> {
        let mut objects = BTreeMap::new();
        let mut total = 0usize;
        for (id, payload) in records {
            total = total
                .checked_add(payload.len())
                .ok_or(Error::Limit("import bytes"))?;
            if total > limits.bytes || objects.len() >= limits.objects {
                return Err(Error::Limit("import budget"));
            }
            let object = Object::decode(id, &payload)?;
            match objects.entry(id) {
                std::collections::btree_map::Entry::Occupied(prior) => {
                    if prior.get() != &object {
                        return Err(invalid("duplicate object collision"));
                    }
                }
                std::collections::btree_map::Entry::Vacant(slot) => {
                    slot.insert(object);
                }
            }
        }
        let mut verifier = Verifier::new(&objects, limits);
        for root in roots {
            verifier.verify(root)?;
        }
        for (id, object) in &objects {
            if let Some(prior) = self.objects.get(id)
                && prior != object
            {
                return Err(invalid("object collision"));
            }
        }
        self.counts.extend(verifier.counts);
        self.roots.extend(verifier.roots);
        self.objects.extend(objects);
        let maps: Vec<_> = self
            .roots
            .keys()
            .filter(|id| matches!(id.kind, 2 | 3))
            .copied()
            .collect();
        for id in maps {
            self.node_metrics(id)?;
        }
        Ok(())
    }

    /// Recheck from canonical bytes, without trusting incremental caches.
    pub fn verify(&self, root: Id, limits: Limits) -> Result<()> {
        Verifier::new(&self.objects, limits).verify(root)
    }

    pub fn read(&self, root: Id, path: &[Vec<u8>]) -> Result<Entry> {
        validate_path(path)?;
        self.root(root)?;
        let mut entry = root_entry(root)?;
        for name in path {
            entry = match entry {
                Entry::Directory(id) => self.lookup(id, &nibbles(name))?,
                Entry::Absent => return Ok(Entry::Absent),
                Entry::Conflict(id) => return Err(Error::Conflict(id)),
                _ => return Err(invalid("non-directory ancestor")),
            };
        }
        Ok(entry)
    }
    fn lookup(&self, mut id: Id, mut key: &[u8]) -> Result<Entry> {
        loop {
            match self.node(id)? {
                Node::Leaf(entries) => {
                    return Ok(entries
                        .binary_search_by(|(k, _)| k.as_slice().cmp(key))
                        .ok()
                        .map(|i| entries[i].1.clone())
                        .unwrap_or(Entry::Absent));
                }
                Node::Branch {
                    prefix,
                    terminal,
                    children,
                    ..
                } => {
                    let Some(rest) = key.strip_prefix(prefix.as_slice()) else {
                        return Ok(Entry::Absent);
                    };
                    if rest.is_empty() {
                        return Ok(terminal.clone().unwrap_or(Entry::Absent));
                    }
                    let Some(child) = children.get(&rest[0]) else {
                        return Ok(Entry::Absent);
                    };
                    id = *child;
                    key = &rest[1..];
                }
            }
        }
    }

    pub fn edit(&mut self, root: Id, edits: &[Edit]) -> Result<Id> {
        self.transform(root, &[], edits)
    }

    pub fn transform(&mut self, root: Id, moves: &[Relocate], edits: &[Edit]) -> Result<Id> {
        self.root(root)?;
        if root.kind == 4 {
            return Err(Error::Conflict(root));
        }
        for action in moves {
            validate_nonempty(&action.source)?;
            validate_nonempty(&action.destination)?;
            if overlaps(&action.source, &action.destination) {
                return Err(invalid("self/ancestor relocation"));
            }
            if self.read(root, &action.source)? != action.expected_source
                || action.expected_source == Entry::Absent
                || self.read(root, &action.destination)? != action.expected_destination
            {
                return Err(Error::Stale);
            }
            if action.expected_destination != Entry::Absent && !action.overwrite {
                return Err(invalid("explicit overwrite required"));
            }
            self.check_entry(&action.expected_source, &action.destination)?;
        }
        for (i, a) in moves.iter().enumerate() {
            for b in &moves[..i] {
                if overlaps(&a.source, &b.source)
                    || overlaps(&a.destination, &b.destination)
                    || strict_overlap(&a.source, &b.destination)
                    || strict_overlap(&a.destination, &b.source)
                {
                    return Err(invalid("overlapping structural requests"));
                }
                // Exact sibling source/destination aliases are simultaneous swaps.
            }
        }
        let mut next = root;
        for action in moves.iter().filter(|m| !m.copy) {
            next = self.update_path(next, &action.source, Entry::Absent)?;
        }
        for action in moves {
            next = self.update_path(next, &action.destination, action.expected_source.clone())?;
        }
        // All leaf witnesses are against this exact post-structural view.
        for (i, edit) in edits.iter().enumerate() {
            validate_nonempty(&edit.path)?;
            if edits[..i].iter().any(|e| overlaps(&e.path, &edit.path)) {
                return Err(invalid("overlapping leaf edits"));
            }
            if self.read(next, &edit.path)? != edit.expected {
                return Err(Error::Stale);
            }
            self.check_entry(&edit.value, &edit.path)?;
        }
        for edit in edits {
            next = self.update_path(next, &edit.path, edit.value.clone())?;
        }
        Ok(next)
    }

    fn check_entry(&self, entry: &Entry, path: &[Vec<u8>]) -> Result<Bounds> {
        let bounds = match entry {
            Entry::Absent => Bounds::default(),
            Entry::File { blob, .. } | Entry::Symlink(blob) => match self.object(*blob)? {
                Object::Blob(_) => Bounds::default(),
                _ => return Err(invalid("expected blob")),
            },
            Entry::Directory(id) => self.root(*id)?,
            Entry::Conflict(id) => {
                match self.object(*id)? {
                    Object::Conflict(conflict) if conflict.region == 0 => {}
                    _ => return Err(invalid("path conflict region")),
                }
                self.bounds(*id)?
            }
        };
        let prefix_bytes = path_bytes(path);
        if path.len() + bounds.depth > 256
            || prefix_bytes + bounds.bytes + usize::from(bounds.depth != 0 && !path.is_empty())
                > 4096
        {
            return Err(invalid("relocated source exceeds path bounds"));
        }
        Ok(bounds)
    }

    fn update_path(&mut self, map: Id, path: &[Vec<u8>], value: Entry) -> Result<Id> {
        let mut ancestors = Vec::with_capacity(path.len());
        let mut current = map;
        for (index, name) in path.iter().enumerate() {
            let key = nibbles(name);
            let parent = current;
            if index + 1 < path.len() {
                current = match self.lookup(current, &key)? {
                    Entry::Directory(id) => id,
                    _ => return Err(invalid("explicit directory parent required")),
                };
            }
            ancestors.push((parent, key));
        }
        let mut entry = value;
        let mut result = map;
        while let Some((parent, key)) = ancestors.pop() {
            result = self.update_node(parent, &key, entry)?;
            // Only changed radix nodes need summaries; siblings retain their
            // trusted counts, metrics and directory/conflict bounds.
            let bounds = self.map_bounds(result)?;
            self.roots.insert(result, bounds);
            entry = Entry::Directory(result);
        }
        Ok(result)
    }

    fn update_node(&mut self, mut id: Id, mut key: &[u8], value: Entry) -> Result<Id> {
        let old = self.lookup(id, key)?;
        if old == value {
            return Ok(id);
        }
        let mut ancestors = Vec::new();
        let mut updated = loop {
            match self.node(id)?.clone() {
                Node::Leaf(mut entries) => {
                    match entries.binary_search_by(|(k, _)| k.as_slice().cmp(key)) {
                        Ok(i) if value == Entry::Absent => {
                            entries.remove(i);
                        }
                        Ok(i) => {
                            entries[i].1 = value;
                        }
                        // A missing key can only be reached for an insertion:
                        // equal/absent updates returned before the descent.
                        Err(i) => entries.insert(i, (key.to_vec(), value)),
                    }
                    break self.build_node(entries)?;
                }
                Node::Branch {
                    count,
                    prefix,
                    mut terminal,
                    mut children,
                } => {
                    let shared = common(&prefix, key);
                    if shared < prefix.len() {
                        let old_slot = prefix[shared];
                        let old = self.store_node(Node::Branch {
                            count,
                            prefix: prefix[shared + 1..].to_vec(),
                            terminal,
                            children,
                        })?;
                        let mut groups = BTreeMap::from([(old_slot, old)]);
                        let new_terminal = if key.len() == shared {
                            Some(value)
                        } else {
                            let child =
                                self.build_node(vec![(key[shared + 1..].to_vec(), value)])?;
                            groups.insert(key[shared], child);
                            None
                        };
                        break self.store_node(Node::Branch {
                            count: count.checked_add(1).ok_or(Error::Limit("map count"))?,
                            prefix: prefix[..shared].to_vec(),
                            terminal: new_terminal,
                            children: groups,
                        })?;
                    }
                    let rest = &key[prefix.len()..];
                    let new_count = if value == Entry::Absent {
                        count - 1
                    } else if old == Entry::Absent {
                        count.checked_add(1).ok_or(Error::Limit("map count"))?
                    } else {
                        count
                    };
                    if rest.is_empty() {
                        terminal = (value != Entry::Absent).then(|| value.clone());
                    } else {
                        let slot = rest[0];
                        if let Some(child) = children.get(&slot).copied() {
                            ancestors.push((new_count, prefix, terminal, children, slot));
                            id = child;
                            key = &rest[1..];
                            continue;
                        }
                        if value != Entry::Absent {
                            let child =
                                self.build_node(vec![(rest[1..].to_vec(), value.clone())])?;
                            children.insert(slot, child);
                        }
                    }
                    break self.finish_branch(new_count, prefix, terminal, children)?;
                }
            }
        };
        while let Some((count, prefix, terminal, mut children, slot)) = ancestors.pop() {
            if self.count(updated)? == 0 {
                children.remove(&slot);
            } else {
                children.insert(slot, updated);
            }
            updated = self.finish_branch(count, prefix, terminal, children)?;
        }
        Ok(updated)
    }

    fn finish_branch(
        &mut self,
        count: u32,
        prefix: Vec<u8>,
        terminal: Option<Entry>,
        children: BTreeMap<u8, Id>,
    ) -> Result<Id> {
        if count <= 32 {
            let mut entries = Vec::new();
            if let Some(entry) = terminal {
                entries.push((prefix.clone(), entry));
            }
            for (slot, child) in children {
                let mut start = prefix.clone();
                start.push(slot);
                self.collect_node(child, &start, &mut entries)?;
            }
            return self.store_node(Node::Leaf(entries));
        }
        if terminal.is_none() && children.len() == 1 {
            let (slot, child) = children.into_iter().next().expect("one child");
            let Node::Branch {
                prefix: child_prefix,
                terminal,
                children,
                ..
            } = self.node(child)?.clone()
            else {
                return Err(invalid("large branch child is leaf"));
            };
            let mut combined = prefix;
            combined.push(slot);
            combined.extend(child_prefix);
            return self.store_node(Node::Branch {
                count,
                prefix: combined,
                terminal,
                children,
            });
        }
        self.store_node(Node::Branch {
            count,
            prefix,
            terminal,
            children,
        })
    }

    fn build_node(&mut self, entries: Vec<(Vec<u8>, Entry)>) -> Result<Id> {
        enum Task {
            Build(Vec<(Vec<u8>, Entry)>),
            Finish(u32, Vec<u8>, Option<Entry>, Vec<u8>),
        }
        let mut pending = vec![Task::Build(entries)];
        let mut results = Vec::new();
        while let Some(task) = pending.pop() {
            match task {
                Task::Build(entries) if entries.len() <= 32 => {
                    results.push(self.store_node(Node::Leaf(entries))?);
                }
                Task::Build(entries) => {
                    let prefix_len = common(&entries[0].0, &entries[entries.len() - 1].0);
                    let prefix = entries[0].0[..prefix_len].to_vec();
                    let count =
                        u32::try_from(entries.len()).map_err(|_| Error::Limit("map count"))?;
                    let mut terminal = None;
                    let mut groups: BTreeMap<u8, Vec<(Vec<u8>, Entry)>> = BTreeMap::new();
                    for (key, value) in entries {
                        if key.len() == prefix_len {
                            terminal = Some(value);
                        } else {
                            groups
                                .entry(key[prefix_len])
                                .or_default()
                                .push((key[prefix_len + 1..].to_vec(), value));
                        }
                    }
                    pending.push(Task::Finish(
                        count,
                        prefix,
                        terminal,
                        groups.keys().copied().collect(),
                    ));
                    pending.extend(groups.into_values().rev().map(Task::Build));
                }
                Task::Finish(count, prefix, terminal, slots) => {
                    let start = results.len() - slots.len();
                    let children = slots.into_iter().zip(results.drain(start..)).collect();
                    results.push(self.store_node(Node::Branch {
                        count,
                        prefix,
                        terminal,
                        children,
                    })?);
                }
            }
        }
        Ok(results.pop().expect("one completed map"))
    }
    fn collect_node(&self, id: Id, prefix: &[u8], out: &mut Vec<(Vec<u8>, Entry)>) -> Result<()> {
        let mut prefix = prefix.to_vec();
        let mut pending = vec![(id, prefix.len(), None)];
        while let Some((id, length, slot)) = pending.pop() {
            prefix.truncate(length);
            prefix.extend(slot);
            match self.node(id)? {
                Node::Leaf(entries) => {
                    for (suffix, entry) in entries {
                        let mut key = prefix.clone();
                        key.extend(suffix);
                        out.push((key, entry.clone()));
                    }
                }
                Node::Branch {
                    prefix: suffix,
                    terminal,
                    children,
                    ..
                } => {
                    prefix.extend(suffix);
                    if let Some(entry) = terminal {
                        out.push((prefix.clone(), entry.clone()));
                    }
                    for (slot, child) in children.iter().rev() {
                        pending.push((*child, prefix.len(), Some(*slot)));
                    }
                }
            }
        }
        Ok(())
    }
    fn map_bounds(&mut self, id: Id) -> Result<Bounds> {
        let metrics = self.node_metrics(id)?;
        if !metrics.nibbles.is_multiple_of(2) {
            return Err(invalid("odd complete map path"));
        }
        let bounds = Bounds {
            depth: metrics.depth,
            bytes: metrics.nibbles / 2,
            unresolved: metrics.unresolved,
        };
        if bounds.depth > 256 || bounds.bytes > 4096 {
            return Err(invalid("source path bounds"));
        }
        Ok(bounds)
    }
    fn node_metrics(&mut self, id: Id) -> Result<Metrics> {
        if let Some(metrics) = self.metrics.get(&id) {
            return Ok(*metrics);
        }
        let mut pending = vec![(id, false)];
        while let Some((current, expanded)) = pending.pop() {
            if self.metrics.contains_key(&current) {
                continue;
            }
            if !expanded && let Node::Branch { children, .. } = self.node(current)? {
                pending.push((current, true));
                pending.extend(children.values().map(|child| (*child, false)));
                continue;
            }
            let mut result = Metrics::default();
            match self.node(current)? {
                Node::Leaf(entries) => {
                    for (key, entry) in entries {
                        let child = self.check_entry(entry, &[])?;
                        result.depth = result.depth.max(1 + child.depth);
                        result.unresolved |= child.unresolved;
                        result.nibbles = result
                            .nibbles
                            .max(key.len() + 2 * (child.bytes + usize::from(child.depth != 0)));
                    }
                }
                Node::Branch {
                    prefix,
                    terminal,
                    children,
                    ..
                } => {
                    if let Some(entry) = terminal {
                        let child = self.check_entry(entry, &[])?;
                        result.depth = 1 + child.depth;
                        result.unresolved |= child.unresolved;
                        result.nibbles =
                            prefix.len() + 2 * (child.bytes + usize::from(child.depth != 0));
                    }
                    for child in children.values() {
                        let metrics = self.metrics.get(child).expect("completed child metrics");
                        result.depth = result.depth.max(metrics.depth);
                        result.unresolved |= metrics.unresolved;
                        result.nibbles = result.nibbles.max(prefix.len() + 1 + metrics.nibbles);
                    }
                }
            }
            self.metrics.insert(current, result);
        }
        Ok(self.metrics[&id])
    }

    /// Immediate bindings only; conflict endpoints remain exact opaque values.
    pub fn list(&self, root: Id, path: &[Vec<u8>]) -> Result<Vec<(Vec<u8>, Entry)>> {
        let entry = self.read(root, path)?;
        let id = match entry {
            Entry::Directory(id) => id,
            Entry::Conflict(id) => return Err(Error::Conflict(id)),
            _ => return Err(invalid("not a directory")),
        };
        let mut bindings = Vec::new();
        self.collect_node(id, &[], &mut bindings)?;
        bindings
            .into_iter()
            .map(|(key, entry)| Ok((bytes_from_nibbles(&key)?, entry)))
            .collect()
    }

    pub fn diff(&self, before: Id, after: Id) -> Result<Vec<Difference>> {
        self.diff_with_limit(before, after, 1_000_000)
    }
    pub fn diff_with_limit(&self, before: Id, after: Id, limit: usize) -> Result<Vec<Difference>> {
        self.root(before)?;
        self.root(after)?;
        let mut output = Vec::new();
        self.diff_entry(
            root_entry(before)?,
            root_entry(after)?,
            &[],
            &mut output,
            limit,
        )?;
        Ok(output)
    }
    fn diff_entry(
        &self,
        before: Entry,
        after: Entry,
        path: &[Vec<u8>],
        output: &mut Vec<Difference>,
        limit: usize,
    ) -> Result<()> {
        // DFS checkpoints keep one path buffer rather than copying every
        // ancestor component into each queued sibling.
        let mut path = path.to_vec();
        let mut pending = vec![(before, after, path.len(), None)];
        while let Some((before, after, length, name)) = pending.pop() {
            path.truncate(length);
            path.extend(name);
            if before == after {
                continue;
            }
            if let (Entry::Directory(a), Entry::Directory(b)) = (&before, &after) {
                let mut changes = Vec::new();
                self.diff_views(
                    View::Node(*a, 0),
                    View::Node(*b, 0),
                    &mut Vec::new(),
                    &mut changes,
                    limit.saturating_sub(output.len()),
                )?;
                for (key, left, right) in changes.into_iter().rev() {
                    pending.push((left, right, path.len(), Some(bytes_from_nibbles(&key)?)));
                }
            } else {
                if output.len() >= limit {
                    return Err(Error::Limit("diff output"));
                }
                output.push(Difference {
                    path: path.clone(),
                    before,
                    after,
                });
            }
        }
        Ok(())
    }
    #[expect(
        clippy::ptr_arg,
        reason = "Preserve the merge-facing traversal signature"
    )]
    fn diff_views(
        &self,
        left: View,
        right: View,
        prefix: &mut Vec<u8>,
        output: &mut Vec<(Vec<u8>, Entry, Entry)>,
        limit: usize,
    ) -> Result<()> {
        let mut prefix = prefix.clone();
        let mut pending = vec![(left, right, prefix.len(), None)];
        while let Some((left, right, length, slot)) = pending.pop() {
            prefix.truncate(length);
            prefix.extend(slot);
            if left == right {
                continue;
            }
            let (a, ac) = self.partition(left)?;
            let (b, mut bc) = self.partition(right)?;
            if a != b {
                if output.len() >= limit {
                    return Err(Error::Limit("changed directory bindings"));
                }
                output.push((prefix.clone(), a, b));
            }
            let mut children = Vec::with_capacity(ac.len() + bc.len());
            for (slot, child) in ac {
                children.push((slot, child, bc.remove(&slot).unwrap_or(View::Empty)));
            }
            children.extend(
                bc.into_iter()
                    .map(|(slot, child)| (slot, View::Empty, child)),
            );
            // Preserve the existing left-partitions-then-right-only DFS order.
            for (slot, left, right) in children.into_iter().rev() {
                pending.push((left, right, prefix.len(), Some(slot)));
            }
        }
        Ok(())
    }
    fn partition(&self, view: View) -> Result<(Entry, BTreeMap<u8, View>)> {
        let entries = match view {
            View::Empty => return Ok((Entry::Absent, BTreeMap::new())),
            View::Leaf(entries) => entries,
            View::Node(id, offset) => match self.node(id)? {
                Node::Leaf(entries) => entries.clone(),
                Node::Branch {
                    prefix,
                    terminal,
                    children,
                    ..
                } => {
                    if offset < prefix.len() {
                        return Ok((
                            Entry::Absent,
                            BTreeMap::from([(prefix[offset], View::Node(id, offset + 1))]),
                        ));
                    }
                    return Ok((
                        terminal.clone().unwrap_or(Entry::Absent),
                        children
                            .iter()
                            .map(|(slot, id)| (*slot, View::Node(*id, 0)))
                            .collect(),
                    ));
                }
            },
        };
        let mut terminal = Entry::Absent;
        let mut groups: BTreeMap<u8, Vec<(Vec<u8>, Entry)>> = BTreeMap::new();
        for (key, entry) in entries {
            if key.is_empty() {
                terminal = entry;
            } else {
                groups
                    .entry(key[0])
                    .or_default()
                    .push((key[1..].to_vec(), entry));
            }
        }
        Ok((
            terminal,
            groups
                .into_iter()
                .map(|(slot, entries)| (slot, View::Leaf(entries)))
                .collect(),
        ))
    }
}

#[derive(Eq, PartialEq)]
enum View {
    Empty,
    Node(Id, usize),
    Leaf(Vec<(Vec<u8>, Entry)>),
}

fn invalid(message: &str) -> Error {
    Error::Invalid(message.to_owned())
}
fn root_entry(id: Id) -> Result<Entry> {
    match id.kind {
        2 | 3 => Ok(Entry::Directory(id)),
        4 => Ok(Entry::Conflict(id)),
        _ => Err(invalid("invalid source root kind")),
    }
}
fn nibbles(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().flat_map(|b| [b >> 4, b & 15]).collect()
}
fn bytes_from_nibbles(nibbles: &[u8]) -> Result<Vec<u8>> {
    if !nibbles.len().is_multiple_of(2) || nibbles.iter().any(|n| *n > 15) {
        return Err(invalid("invalid complete nibble key"));
    }
    Ok(nibbles
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| pair[0] << 4 | pair[1])
        .collect())
}
fn common(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).take_while(|(a, b)| a == b).count()
}
fn overlaps(a: &[Vec<u8>], b: &[Vec<u8>]) -> bool {
    a.starts_with(b) || b.starts_with(a)
}
fn strict_overlap(a: &[Vec<u8>], b: &[Vec<u8>]) -> bool {
    a != b && overlaps(a, b)
}
fn path_bytes(path: &[Vec<u8>]) -> usize {
    path.iter().map(Vec::len).sum::<usize>() + path.len().saturating_sub(1)
}
pub fn validate_name(name: &[u8]) -> Result<()> {
    if name.is_empty()
        || name.len() > 255
        || name == b"."
        || name == b".."
        || name.iter().any(|b| *b == 0 || *b == b'/')
    {
        return Err(invalid("invalid source basename"));
    }
    Ok(())
}
pub fn validate_path(path: &[Vec<u8>]) -> Result<()> {
    if path.len() > 256 || path_bytes(path) > 4096 {
        return Err(invalid("source path bounds"));
    }
    for name in path {
        validate_name(name)?;
    }
    Ok(())
}
fn validate_nonempty(path: &[Vec<u8>]) -> Result<()> {
    validate_path(path)?;
    if path.is_empty() {
        return Err(invalid("root edit requires explicit resolution"));
    }
    Ok(())
}

struct Verifier<'a> {
    objects: &'a BTreeMap<Id, Object>,
    limits: Limits,
    visited: BTreeSet<Id>,
    active: BTreeSet<Id>,
    counts: BTreeMap<Id, u32>,
    roots: BTreeMap<Id, Bounds>,
    bindings: usize,
    bytes: usize,
}
impl<'a> Verifier<'a> {
    fn new(objects: &'a BTreeMap<Id, Object>, limits: Limits) -> Self {
        Self {
            objects,
            limits,
            visited: BTreeSet::new(),
            active: BTreeSet::new(),
            counts: BTreeMap::new(),
            roots: BTreeMap::new(),
            bindings: 0,
            bytes: 0,
        }
    }
    fn object(&mut self, id: Id) -> Result<&'a Object> {
        let object = self.objects.get(&id).ok_or(Error::NeedObjects(id))?;
        if self.visited.insert(id) {
            if self.visited.len() > self.limits.objects {
                return Err(Error::Limit("verification objects"));
            }
            self.bytes = self
                .bytes
                .checked_add(object.encode()?.len())
                .ok_or(Error::Limit("verification bytes"))?;
            if self.bytes > self.limits.bytes {
                return Err(Error::Limit("verification bytes"));
            }
            if object.id()? != id {
                return Err(invalid("object identity mismatch"));
            }
        }
        Ok(object)
    }
    fn verify(&mut self, root: Id) -> Result<()> {
        // Revision ancestry is iterative: history length is not source depth.
        let mut pending = vec![root];
        let mut seen = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            match self.object(id)? {
                Object::Revision(revision) => {
                    self.source_root(revision.root)?;
                    pending.extend(&revision.parents);
                }
                Object::Map(_) | Object::Conflict(_) => {
                    self.source_root(id)?;
                }
                _ => return Err(invalid("invalid protected root")),
            }
        }
        Ok(())
    }
    fn source_root(&mut self, id: Id) -> Result<Bounds> {
        match self.object(id)? {
            Object::Map(_) => {}
            Object::Conflict(conflict) if conflict.region == 1 => {}
            _ => return Err(invalid("invalid source-root type/region")),
        }
        self.source(id, 0)
    }
    fn source(&mut self, id: Id, depth: usize) -> Result<Bounds> {
        enum Task {
            Visit(Id, usize),
            Finish(Id, bool, Vec<(usize, Entry)>),
        }
        let mut pending = vec![Task::Visit(id, depth)];
        while let Some(task) = pending.pop() {
            match task {
                Task::Visit(current, depth) => {
                    if depth > 256 {
                        return Err(invalid("source nesting bound"));
                    }
                    if self.roots.contains_key(&current) {
                        continue;
                    }
                    if !self.active.insert(current) {
                        return Err(invalid("owning cycle"));
                    }
                    let (is_map, entries) = match self.object(current)? {
                        Object::Map(_) => {
                            let mut entries = Vec::new();
                            self.map(current, &[], &mut entries)?;
                            (
                                true,
                                entries
                                    .into_iter()
                                    .map(|(name, entry)| (name.len(), entry))
                                    .collect::<Vec<_>>(),
                            )
                        }
                        Object::Conflict(conflict) => (
                            false,
                            conflict
                                .positive
                                .iter()
                                .chain(&conflict.negative)
                                .map(|(entry, _)| (0, entry.clone()))
                                .collect(),
                        ),
                        _ => return Err(invalid("expected source value")),
                    };
                    let mut dependencies = Vec::new();
                    for (_, entry) in &entries {
                        if let Some(child) = self.entry_source(entry)? {
                            dependencies.push(Task::Visit(child, depth + usize::from(is_map)));
                        }
                    }
                    pending.push(Task::Finish(current, is_map, entries));
                    pending.extend(dependencies.into_iter().rev());
                }
                Task::Finish(current, is_map, entries) => {
                    let mut result = Bounds {
                        unresolved: !is_map,
                        ..Bounds::default()
                    };
                    for (name_len, entry) in entries {
                        let child = match entry {
                            Entry::Directory(id) | Entry::Conflict(id) => self.roots[&id],
                            _ => Bounds::default(),
                        };
                        result.depth = result.depth.max(usize::from(is_map) + child.depth);
                        result.unresolved |= child.unresolved;
                        result.bytes = result
                            .bytes
                            .max(name_len + child.bytes + usize::from(is_map && child.depth != 0));
                    }
                    if result.depth > 256 || result.bytes > 4096 {
                        return Err(invalid("source path bounds"));
                    }
                    self.active.remove(&current);
                    self.roots.insert(current, result);
                }
            }
        }
        Ok(self.roots[&id])
    }
    fn entry_source(&mut self, entry: &Entry) -> Result<Option<Id>> {
        match entry {
            Entry::Absent => Ok(None),
            Entry::File { blob, .. } | Entry::Symlink(blob) => match self.object(*blob)? {
                Object::Blob(_) => Ok(None),
                _ => Err(invalid("expected blob dependency")),
            },
            Entry::Directory(id) => {
                if !matches!(self.object(*id)?, Object::Map(_)) {
                    return Err(invalid("directory reference kind"));
                }
                Ok(Some(*id))
            }
            Entry::Conflict(id) => {
                match self.object(*id)? {
                    Object::Conflict(conflict) if conflict.region == 0 => {}
                    _ => return Err(invalid("path conflict region")),
                }
                Ok(Some(*id))
            }
        }
    }
    fn map(&mut self, id: Id, prefix: &[u8], output: &mut Vec<(Vec<u8>, Entry)>) -> Result<u32> {
        enum Task {
            Visit(Id, usize, Option<u8>),
            Finish(Id, u32, u32, Vec<Id>),
        }
        let mut prefix = prefix.to_vec();
        let mut pending = vec![Task::Visit(id, prefix.len(), None)];
        let mut active = BTreeSet::new();
        while let Some(task) = pending.pop() {
            match task {
                Task::Visit(current, length, slot) => {
                    prefix.truncate(length);
                    prefix.extend(slot);
                    if prefix.len() > 510 {
                        return Err(invalid("radix key bound"));
                    }
                    if !active.insert(current) {
                        return Err(invalid("owning cycle"));
                    }
                    match self.object(current)? {
                        Object::Map(Node::Leaf(entries)) => {
                            for (suffix, entry) in entries {
                                let mut key = prefix.clone();
                                key.extend(suffix);
                                self.map_binding(&key, entry, output)?;
                            }
                            self.counts.insert(current, entries.len() as u32);
                            active.remove(&current);
                        }
                        Object::Map(Node::Branch {
                            count,
                            prefix: suffix,
                            terminal,
                            children,
                        }) => {
                            prefix.extend(suffix);
                            if prefix.len() > 510 {
                                return Err(invalid("radix key bound"));
                            }
                            if let Some(entry) = terminal {
                                self.map_binding(&prefix, entry, output)?;
                            }
                            pending.push(Task::Finish(
                                current,
                                *count,
                                u32::from(terminal.is_some()),
                                children.values().copied().collect(),
                            ));
                            for (slot, child) in children.iter().rev() {
                                pending.push(Task::Visit(*child, prefix.len(), Some(*slot)));
                            }
                        }
                        _ => return Err(invalid("wrong map dependency kind")),
                    }
                }
                Task::Finish(current, count, mut total, children) => {
                    for child in children {
                        let size = self.counts[&child];
                        if size == 0 {
                            return Err(invalid("empty radix partition"));
                        }
                        total = total.checked_add(size).ok_or(Error::Limit("map count"))?;
                    }
                    if total != count {
                        return Err(invalid("forged map count"));
                    }
                    self.counts.insert(current, count);
                    active.remove(&current);
                }
            }
        }
        Ok(self.counts[&id])
    }

    fn map_binding(
        &mut self,
        key: &[u8],
        entry: &Entry,
        output: &mut Vec<(Vec<u8>, Entry)>,
    ) -> Result<()> {
        let name = bytes_from_nibbles(key)?;
        validate_name(&name)?;
        self.bindings = self
            .bindings
            .checked_add(1)
            .ok_or(Error::Limit("verification bindings"))?;
        if self.bindings > self.limits.bindings {
            return Err(Error::Limit("verification bindings"));
        }
        output.push((name, entry.clone()));
        Ok(())
    }
}
