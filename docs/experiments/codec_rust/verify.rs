//! Owning-closure and context validation. Audit IDs never cause a fetch.
use crate::maps::{self, FlatMap, Store};
use crate::wire::{Encode, Entry, Id, Object, Result, decode, name_valid};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Default)]
struct Context {
    components: usize,
    bytes: usize,
    conflicts: usize,
}
impl Context {
    fn child(self, name: &[u8]) -> Result<Self> {
        let child = Self {
            components: self.components + 1,
            bytes: self.bytes + usize::from(self.components != 0) + name.len(),
            ..self
        };
        if child.components > 256 || child.bytes > 4096 {
            return Err("source path bounds".into());
        }
        Ok(child)
    }
}

pub struct Verifier<'a> {
    store: &'a Store,
    active: BTreeSet<Id>,
    visits: usize,
}
impl<'a> Verifier<'a> {
    pub fn check(store: &'a Store, root: Id) -> Result<()> {
        Self {
            store,
            active: BTreeSet::new(),
            visits: 0,
        }
        .walk(root, Context::default(), false)
    }
    fn enter(&mut self, id: Id) -> Result<()> {
        self.visits += 1;
        if self.visits > 30_000 || self.active.len() >= 256 {
            return Err("reference resource budget".into());
        }
        if !self.active.insert(id) {
            return Err("reference cycle".into());
        }
        Ok(())
    }
    fn load(&self, id: Id) -> Result<(Object, BTreeSet<Id>)> {
        let payload = self.store.get(&id).ok_or("missing owning dependency")?;
        if Id::hash(id.kind, payload) != id {
            return Err("object hash mismatch".into());
        }
        let parsed = decode(id.kind, payload)?;
        if parsed.0.encoded()? != *payload {
            return Err("noncanonical round trip".into());
        }
        Ok(parsed)
    }
    fn entry(&mut self, entry: &Entry, context: Context, require_directory: bool) -> Result<()> {
        if require_directory && !matches!(entry, Entry::Directory(_) | Entry::Conflict(_)) {
            return Err("directory-valued alternative required".into());
        }
        self.walk(entry.id(), context, require_directory)
    }
    fn walk(&mut self, id: Id, context: Context, strict_directory: bool) -> Result<()> {
        if matches!(id.kind, 2 | 3) {
            let flat = self.map(id, &[], context)?;
            let rebuilt = maps::build(&flat, &mut Store::new())?;
            if rebuilt != id {
                return Err("noncanonical radix normalization".into());
            }
            return Ok(());
        }
        self.enter(id)?;
        let (object, audit) = self.load(id)?;
        match object {
            Object::Blob(_) => (),
            Object::State(entry) => self.entry(&entry, Context::default(), true)?,
            Object::Revision {
                state,
                parents,
                provenance,
                ..
            } => {
                self.walk(state, Context::default(), false)?;
                if let Some(provenance) = provenance {
                    let (record, audit) = self.load(provenance)?;
                    match record {
                        Object::Provenance { result, .. }
                            if result == state && !audit.contains(&id) => {}
                        _ => return Err("revision provenance result/output binding".into()),
                    }
                    self.walk(provenance, Context::default(), false)?;
                }
                for parent in parents {
                    self.walk(parent, Context::default(), false)?;
                }
            }
            Object::Conflict {
                region,
                base,
                alternatives,
            } => {
                let context = Context {
                    conflicts: context.conflicts + 1,
                    ..context
                };
                if context.conflicts > 64 {
                    return Err("conflict nesting".into());
                }
                if strict_directory && region != 1 {
                    return Err("root/nested conflict region".into());
                }
                if let Some(base) = base {
                    self.entry(&base, context, region == 1)?;
                }
                for entry in alternatives {
                    if let Some(entry) = entry {
                        self.entry(&entry, context, region == 1)?;
                    } else if strict_directory {
                        return Err("absent directory alternative".into());
                    }
                }
            }
            Object::Provenance { result, .. } => {
                if audit.contains(&id) {
                    return Err("self audit commitment".into());
                }
                self.walk(result, Context::default(), false)?;
            }
            Object::Leaf(_) | Object::Branch { .. } => return Err("map dispatch invariant".into()),
        }
        self.active.remove(&id);
        Ok(())
    }
    fn map(&mut self, id: Id, prefix: &[u8], context: Context) -> Result<FlatMap> {
        self.enter(id)?;
        let (object, _) = self.load(id)?;
        let mut values = FlatMap::new();
        match object {
            Object::Leaf(items) => {
                for (tail, value) in items {
                    let key = complete_name(prefix, &tail)?;
                    self.entry(&value, context.child(&key)?, false)?;
                    if values.insert(key, value).is_some() {
                        return Err("duplicate map key".into());
                    }
                }
            }
            Object::Branch {
                prefix: common,
                terminal,
                children,
            } => {
                let mut full = prefix.to_vec();
                full.extend(common);
                if full.len() > 510 {
                    return Err("radix prefix bound".into());
                }
                if let Some(value) = terminal {
                    let key = complete_name(&full, &[])?;
                    self.entry(&value, context.child(&key)?, false)?;
                    values.insert(key, value);
                }
                for (slot, child) in children {
                    full.push(slot);
                    let partial = self.map(child, &full, context)?;
                    if partial.is_empty() {
                        return Err("empty branch child".into());
                    }
                    for (key, value) in partial {
                        if values.insert(key, value).is_some() {
                            return Err("overlapping map keys".into());
                        }
                    }
                    full.pop();
                }
            }
            _ => return Err("non-map dependency".into()),
        }
        self.active.remove(&id);
        Ok(values)
    }
}
fn complete_name(prefix: &[u8], suffix: &[u8]) -> Result<Vec<u8>> {
    let n = prefix.len() + suffix.len();
    if n > 510 || !n.is_multiple_of(2) {
        return Err("incomplete/oversize byte name".into());
    }
    let nibbles: Vec<_> = prefix.iter().chain(suffix).copied().collect();
    let bytes: Vec<_> = nibbles
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| pair[0] << 4 | pair[1])
        .collect();
    if !name_valid(&bytes) {
        return Err("invalid byte name".into());
    }
    Ok(bytes)
}
