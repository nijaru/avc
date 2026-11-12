//! Local Git translation. Origin records are retained authority about a foreign
//! boundary; generated export mappings are deterministic derived data, not refs.
use super::*;
use crate::{
    Revision,
    git::{GitCommit, GitOid, GitPlan, GitStore},
};

/// Exact foreign boundary of a deterministic, zero-native-parent import anchor.
/// `parents` are foreign OIDs, not native ancestry or a promise of availability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitOrigin {
    pub revision: Id,
    pub commit: GitOid,
    pub tree: GitOid,
    pub parents: Vec<GitOid>,
    pub raw_commit: Vec<u8>,
}

impl Repository {
    /// Import one exact local SHA-1 Git commit into an untouched native store.
    /// Identity is the exact algorithm-tagged Git commit, not a mutable ref or
    /// directory name. An acknowledged retry needs no surviving foreign store.
    pub fn import_git(
        &mut self,
        git_dir: &Path,
        commit: GitOid,
        request_key: &str,
    ) -> Result<Receipt> {
        self.ensure_idle()?;
        check_request_key(request_key)?;
        if let Some(prior) = self.receipt(request_key)? {
            return self.reconcile_git_import(prior, commit);
        }
        if self.status()?.generation != 0 {
            if let Some(prior) = self.receipt(request_key)? {
                return self.reconcile_git_import(prior, commit);
            }
            return Err(Error::NotEmpty);
        }
        let git = GitStore::open(git_dir)?;
        let foreign = git.read_commit(commit)?;
        let mut engine = Engine::new();
        let root = git.import_source(foreign.tree, &mut engine)?;
        let revision = engine.store(Object::Revision(anchor(&foreign, root)))?;
        let origin = GitOrigin {
            revision,
            commit,
            tree: foreign.tree,
            parents: foreign.parents,
            raw_commit: foreign.raw,
        };
        let records = collect_closure([revision], |id| encode_bounded(engine.object(id)?))?;
        engine.verify(revision, limits())?;
        let prepared = self.prepare_objects(records)?;
        self.publish(
            0,
            request_key,
            prepared,
            Mutation::ImportGit {
                root,
                origin: &origin,
            },
        )
    }

    pub(super) fn reconcile_git_import(&self, prior: Receipt, commit: GitOid) -> Result<Receipt> {
        if prior.before.generation != 0 || prior.operation != (Operation::ImportGit { commit }) {
            return Err(Error::RequestKeyReuse(prior.request_key));
        }
        // The receipt contains the complete request identity. Recover the ACK
        // independently of later storage health; origin inspection/export still
        // refuses damaged records instead of repairing or rebinding them.
        Ok(prior)
    }

    /// Read retained boundary metadata, not physical Git availability. Checks
    /// its checksum and binding to the immutable native anchor's ChangeId.
    pub fn git_origin(&self, revision: Id) -> Result<Option<GitOrigin>> {
        self.ensure_idle()?;
        read_origin(&self.connection, revision)
    }

    /// Inspect every known Git boundary in an exact native history graph.
    /// Bounds metadata traversal without loading all historical file bytes.
    pub fn git_origins(&self, revision: Id) -> Result<Vec<GitOrigin>> {
        self.ensure_idle()?;
        let mut seen = BTreeSet::new();
        let mut pending = vec![revision];
        let mut origins = Vec::new();
        let mut bytes = 0;
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            if seen.len() > MAX_OBJECTS {
                return Err(Error::Limit("history records"));
            }
            let payload = self.read_object(id)?;
            budget(&mut bytes, payload.len())?;
            let Object::Revision(value) = Object::decode(id, &payload)? else {
                return Err(invalid("Git origin traversal requires revisions"));
            };
            pending.extend(value.parents);
            if let Some(origin) = self.git_origin(id)? {
                budget(&mut bytes, origin.raw_commit.len())?;
                origins.push(origin);
            }
        }
        Ok(origins)
    }

    /// Export an exact authored native graph as local candidate Git objects.
    /// Stops only at retained origins, never inventing a Git root when an origin
    /// was not transferred. All representability and mapping checks precede any
    /// Git writes. Later I/O failure can leave unreferenced immutable objects;
    /// neither native selection nor any Git ref/index/worktree is changed.
    pub fn export_git(&mut self, git_dir: &Path, revision: Id) -> Result<GitOid> {
        self.ensure_idle()?;
        let records = collect_closure([revision], |id| self.read_object(id))?;
        let mut engine = Engine::new();
        engine.import(records, revision, limits())?;
        let git = GitStore::open(git_dir)?;
        let mut plan = GitPlan::new();
        let mut mapped = BTreeMap::new();
        let mut translations = BTreeMap::new();
        let mut seen = BTreeSet::new();
        let mut pending = vec![(revision, false)];
        let mut origin_bytes = 0;
        while let Some((id, finish)) = pending.pop() {
            let Object::Revision(value) = engine.object(id)? else {
                return Err(invalid("Git export requires a revision"));
            };
            if finish {
                let parents = value
                    .parents
                    .iter()
                    .map(|id| {
                        mapped
                            .get(id)
                            .copied()
                            .ok_or_else(|| invalid("unmapped Git parent"))
                    })
                    .collect::<Result<Vec<_>>>()?;
                // Source trees were prepared before descending into parents.
                let oid = plan.commit(&git, id, value, &parents)?;
                check_export_mapping(&self.connection, id, oid)?;
                mapped.insert(id, oid);
                translations.insert(id, oid);
            } else if seen.insert(id) {
                if let Some(origin) = self.git_origin(id)? {
                    budget(&mut origin_bytes, origin.raw_commit.len())?;
                    let original = git.read_commit(origin.commit)?;
                    if original.raw != origin.raw_commit
                        || original.tree != origin.tree
                        || original.parents != origin.parents
                        || Object::Revision(anchor(&original, value.root)).id()? != id
                    {
                        return Err(invalid(
                            "retained Git origin does not match original commit/anchor",
                        ));
                    }
                    mapped.insert(id, origin.commit);
                } else {
                    if value.parents.is_empty() {
                        return Err(invalid(
                            "Git export requires a retained foreign-origin boundary; refusing to invent a Git root",
                        ));
                    }
                    // Preflight source before any writes, even for a revision
                    // already translated on a previous export.
                    plan.tree(&git, &engine, value.root)?;
                    pending.push((id, true));
                    pending.extend(value.parents.iter().map(|id| (*id, false)));
                }
            }
        }
        let oid = mapped
            .get(&revision)
            .copied()
            .ok_or_else(|| invalid("missing Git candidate"))?;
        plan.write(&git)?;
        // This transaction owns only a rebuildable mapping. Git objects were
        // installed first; no cross-store atomic/ref-publication claim is made.
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        for (id, translated) in translations {
            check_export_mapping(&tx, id, translated)?;
            tx.execute(
                "INSERT OR IGNORE INTO git_exports VALUES (?1, 1, 1, ?2)",
                params![id.to_bytes()?.as_slice(), translated.to_bytes().as_slice()],
            )?;
        }
        tx.commit()?;
        durability_fence(&self.connection)?;
        Ok(oid)
    }
}

fn change(commit: GitOid) -> [u8; 16] {
    let bytes = commit.to_bytes();
    let mut hash = blake3::Hasher::new();
    hash.update(b"AVC-GIT-ANCHOR\0v1\0");
    hash.update(&[bytes[0], 20]);
    hash.update(&bytes[1..]);
    let mut change = [0; 16];
    change.copy_from_slice(&hash.finalize().as_bytes()[..16]);
    change
}
fn anchor(commit: &GitCommit, root: Id) -> Revision {
    Revision {
        root,
        parents: Vec::new(),
        change: change(commit.oid),
        author: commit.author.clone(),
        message: commit.message.clone(),
        time_ns: commit.time_ns,
        timezone_minutes: commit.timezone_minutes,
    }
}
fn parent_bytes(origin: &GitOrigin) -> Vec<u8> {
    origin
        .parents
        .iter()
        .flat_map(|oid| oid.to_bytes())
        .collect()
}
fn checksum(origin: &GitOrigin) -> Result<[u8; 32]> {
    let mut hash = blake3::Hasher::new();
    hash.update(b"AVC-GIT-ORIGIN\0v1\0");
    hash.update(&origin.revision.to_bytes()?);
    hash.update(&origin.commit.to_bytes());
    hash.update(&origin.tree.to_bytes());
    hash.update(&(origin.parents.len() as u64).to_le_bytes());
    hash.update(&parent_bytes(origin));
    hash.update(&origin.raw_commit);
    Ok(*hash.finalize().as_bytes())
}
pub(super) fn insert_origin(connection: &Connection, origin: &GitOrigin) -> Result<()> {
    if let Some(previous) = read_origin(connection, origin.revision)? {
        if previous != *origin {
            return Err(invalid("Git origin collision"));
        }
    } else {
        connection.execute(
            "INSERT INTO git_origins VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                origin.revision.to_bytes()?.as_slice(),
                origin.commit.to_bytes().as_slice(),
                origin.tree.to_bytes().as_slice(),
                parent_bytes(origin),
                &origin.raw_commit,
                checksum(origin)?.as_slice()
            ],
        )?;
    }
    Ok(())
}
fn read_origin(connection: &Connection, revision: Id) -> Result<Option<GitOrigin>> {
    type StoredOrigin = (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>);
    let row: Option<StoredOrigin> = connection.query_row(
        "SELECT commit_oid, tree_oid, parents, raw_commit, checksum FROM git_origins WHERE revision = ?1",
        [revision.to_bytes()?.as_slice()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
    ).optional()?;
    if row.is_none() && connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM operations WHERE kind = 'import-git' AND after_revision = ?1)",
        [revision.to_bytes()?.as_slice()], |r| r.get::<_, bool>(0))? {
        return Err(invalid("missing retained Git origin"));
    }
    row.map(|(commit, tree, parents, raw_commit, digest)| {
        if parents.len() % 21 != 0 || parents.len() > 16 * 21 || raw_commit.len() > MAX_OBJECT_BYTES
        {
            return Err(invalid("Git origin bounds"));
        }
        let origin = GitOrigin {
            revision,
            commit: GitOid::from_bytes(&commit)?,
            tree: GitOid::from_bytes(&tree)?,
            parents: parents
                .as_chunks::<21>()
                .0
                .iter()
                .map(|bytes| GitOid::from_bytes(bytes))
                .collect::<Result<_>>()?,
            raw_commit,
        };
        if checksum(&origin)?.as_slice() != digest {
            return Err(invalid("Git origin checksum"));
        }
        let native = history::read_revision(connection, revision)?;
        if !native.parents.is_empty() || native.change != change(origin.commit) {
            return Err(invalid("Git origin is not bound to a native import anchor"));
        }
        Ok(origin)
    })
    .transpose()
}
fn check_export_mapping(connection: &Connection, revision: Id, oid: GitOid) -> Result<()> {
    let existing: Option<Vec<u8>> = connection.query_row(
        "SELECT commit_oid FROM git_exports WHERE revision = ?1 AND algorithm = 1 AND policy = 1",
        [revision.to_bytes()?.as_slice()], |r| r.get(0)).optional()?;
    if let Some(existing) = existing
        && GitOid::from_bytes(&existing)? != oid
    {
        return Err(invalid(
            "retained Git export mapping disagrees with deterministic translation",
        ));
    }
    Ok(())
}
