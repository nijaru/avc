# AVC: native source control reference architecture

**Revised 2026-10-14. Status: selected reference design, not validated or frozen.**  
**Owner:** this document defines the proposed behavior; [research.md](research.md) owns evidence and alternatives, and [roadmap.md](roadmap.md) owns the work and proof gates.

## 1. Product requirements and system boundary

Build a safe, fast VCS that works for one developer on an ordinary filesystem and can also serve a repository inspected, edited and integrated by many concurrent agents. It must preserve source data, work offline when the required objects are local, make changes/history understandable, avoid redundant materialization, and publish shared source only through explicit coordination.

This is **not** Git with extra commands. Git and Jujutsu are interoperability and performance baselines. Nor is it an agent runtime, scheduler, CI service, package manager or semantic source database.

The six fundamental operations are: **observe and recover local work; record an authored revision; inspect any revision without checkout; compose revisions with explicit conflicts; synchronize immutable objects; conditionally advance shared targets.** Source integrity and ordinary ergonomics take precedence over theoretical novelty.

The leading representation is a **parent-relative changeset DAG with derived persistent source views**. It is chosen for the reference because two substantial production systems—Scion and Mononoke/Bonsai—demonstrate its advantages for cheap admission and independently evolvable read indexes. It is not declared universally superior to snapshot-root commits. A snapshot-first system with the same APIs and storage is a mandatory comparison before freezing the native format.

## 2. Authoritative data: small and explicit

The canonical closure is **changesets and referenced file-content objects**, plus the durable mutable roots selecting history. Source-tree manifests, blame and path-history indexes are **derived**, not an independently writable source of truth.

Illustrative logical records, **not** frozen Rust serialization:

    ChangeSet {
      format_version,
      parents: Vec<ChangeSetId>,        // ordered; parents[0] is the primary parent
      logical_change_id: ChangeId,     // stable user-visible identity, not a patch algebra
      file_changes: SortedMap<Path, FileChange>,
      author, description, authored_time,
      source_metadata
    }

    FileChange =
        Set { file_kind, content_id, executable_or_mode }
      | Delete
      | Conflict { conflict_object_id }

    Conflict {
      base_entries, side_entries, source_changesets, resolution_context
    }

    WorkspaceRoot {
      workspace_id, generation, head_changeset,
      materialization_state, ownership_epoch
    }

    TargetRoot {
      name, generation, head_changeset, policy_epoch
    }

    ResourceOperation {
      request_identity, affected_resource_generations,
      previous_values, new_values, outcome
    }

The root changeset has no parent and applies its file changes to an empty source namespace. Each later changeset's **exact logical source** is defined by recursively obtaining the first parent's state and applying the listed path modifications, in canonical order. Other parents are ancestry/provenance; they do **not** implicitly alter source bytes. A merge therefore records the changes needed to turn the selected first-parent tree into the agreed result, even when those changes came from its additional parents.

Paths and source entries have versioned portable rules. Initial semantics: regular file, executable file, symlink, and typed unresolved conflict; directories are implied by their child paths. Empty-directory tracking, opaque platform metadata, hard-link identity, and efficient subtree-move operations need explicit future decisions. Reject invalid file/directory collisions or unsupported paths rather than silently reinterpret them.

Canonical entries are sorted, unique and bounded; each Set references an immutable content hash, and Delete is meaningful relative to its parent. File copy/rename information may be recorded as verified provenance, but inode or matching content does not prove user intent. A directory rename can touch many paths in a flat changeset; compare this cost against subtree-copy and snapshot-tree primitives before stabilizing the format. Mononoke supports explicit subtree changes, illustrating a plausible—but nontrivial—way to address it.

An authored revision's **ChangesetId** hashes its versioned canonical metadata, parents and file operations. Equal source bytes reached through different history may yield different ChangesetIds but equal logical source states. A stable **ChangeId** connects rewrites/amendments without implying that two revisions commute. Split/fold creates explicit evolution relationships; conflicting successors remain distinct. Rebase creates a new changeset and may preserve the logical ChangeId; the original remains immutable.

This system does **not** require a second WorkVersion/Application graph. An exact merge result, its selected input identities and algorithm/resolution provenance are recorded by the merge changeset and integration candidate. Do not store redundant mutable versions of the same authoritative state.

## 3. Derived source trees: fast reads without a second authority

The fundamental query is:

    source_view(change_id, index_format_version) -> persistent tree root

For a cached source view, return the immutable tree immediately. Otherwise obtain the first parent's view, apply the changed paths to a persistent map, validate structure, and cache the derived root. Iterative derivation from a retained earlier checkpoint must yield the same logical tree. A cached index is versioned by the canonical interpreter and its own tree encoding; deletion and recomputation must not change file bytes or history.

**A changeset may be durably admitted before its tree is computed.** Clients requesting that tree must either derive/wait and return a complete result, or return an explicit unavailable/in-progress error; never imply a partial tree is complete. In a small local repository derive eagerly when it improves user latency. Hosted fleets can defer rare revisions and compute popular ones on demand or proactively. This is an execution policy, not a change in canonical semantics.

The initial derived tree is a hierarchical Merkle directory map with small sorted flat objects. It supports subtree equality and recursive diffs. Very wide directories may benefit from deterministic paged B+/POS/prolly maps or a compressed Merkle radix trie; choose by update/diff/listing cost, cold-cache locality, adversarial keys and node count. Tree-object hash identity depends on its **versioned encoding**; changing indexes may alter derived root IDs without altering ChangesetIds. Do not promise representation-independent tree hashes unless the encoding guarantees it.

Derived indexes can include ancestry generations/skip links, path-history postings and changed-path Bloom filters, copy candidates, blame/line-origin ranges, and optional symbol search. They use immutable input identities and algorithm/version keys. They must not become alternate source truth. Expensive derivations require bounded work, duplicate suppression, caching, cancellation and observable failure states.

**Cost trade-off:** admitting a delta touching k paths avoids synchronously constructing all affected tree nodes. A future read may pay that deferred work. If a desired historical view has no cached ancestor, reconstruction can require traversing many changesets. Set a materialization/checkpoint policy and benchmark complete write + first read + rebuild + garbage-collection costs. This is why the snapshot-first challenger remains serious.

## 4. Editing, merging, undo and conflicts

**Editing:** a human or agent works in an ordinary or isolated working directory. File watching and status locate candidate path edits; source content and expected parent entries are read/verified. Creating a changeset stages canonical blobs, computes exact Set/Delete operations relative to the selected primary parent, and publishes the changeset with workspace generation CAS. No independent Git index or background public commit is moved.

**Checkout-free editing:** source clients may submit changed paths and expected before-content IDs directly without materializing the repository. An entire request either creates one well-defined new revision or returns a typed stale/conflict result.

**Merging:** obtain the relevant complete derived trees, determine merge bases and compute a deterministic candidate with a well-defined driver policy. Start with three-way tree and conservative text merge; file kind changes, deletion/modification and unmergeable binaries produce typed conflicts. A resulting merge changeset has the selected first parent plus additional parents, and its **first-parent file delta encodes the exact resulting tree**. The second parent's history alone cannot be interpreted as source updates. Multiple bases require explicit virtual-base semantics; arbitrary merge order is not assumed associative.

**Conflicts:** preserve base and alternatives as canonical typed objects; private conflicted states may be recorded and further merged. Normal resolved Git exports and protected land operations reject unresolved conflicts. Structural/text-token/LLM-assisted drivers may propose better resolutions, but must be versioned, bounded, byte-preserving outside intended edits, and separately evaluated for incorrect automatic merges. Model output is never an implicit canonical storage transformation.

**Undo:** source-history operation undo restores an earlier resource root under a current-generation check. Content revert creates an inverse-content revision on a new base and can conflict. Causal selective undo (remove only one author's independently identical effect) is a distinct alternative that this changeset model does **not** guarantee. Jujutsu's documented same-change trade-offs and Darcs/Pijul patch theory are prior art; claiming more would require a precise algebra and tests. A reverted change may be intentionally reapplied; never use an eternal set of ChangeIds as a duplicate filter.

## 5. Private recovery and safe workspaces

A **recovery capture** protects admitted local filesystem bytes under a bounded retention policy. It is not automatically a permanent changeset in shared semantic history. Capture may store a private derived tree or a bounded incremental overlay; measure overhead before choosing. Automatic capture neither pushes nor invokes CI, review, or indexing events.

Watchers, stat metadata and editor events are hints only; recrawl when uncertain. A directory scan is not an atomic multi-file snapshot when editors continue writing. Unsaved buffers cannot be captured by a filesystem watcher. Report the last durable capture, admitted paths and failures. Do not silently retain ignored files, secrets or all untracked outputs. Ignoring a previously captured secret does not erase it from old objects or backups.

**Independent workers get distinct writable directories** sharing immutable objects, caches and safe build artifacts. Workspaces have separate resource generations; advancing workspace A must not falsely conflict with B. The same human/agent may own several sessions, so actor, session, workspace and source-root identities remain distinct.

A workspace is **not** a process/credential/network sandbox. An API fence cannot prevent an already running editor from writing through an open file handle.

**Restore safety:** absent actual exclusion of external filesystem writers, restore to a fresh directory by default. An in-place operation can check dirtiness and preserve before-images, but a check immediately before replacement cannot guarantee no-clobber against an uncontrolled writer. The source transaction and filesystem materialization are two different completion states. Do not claim that metadata atomicity makes checkout atomic.

## 6. Local transaction, object integrity and GC

The local repository has one authority for committed **mutable resource roots and request outcomes**. It does not need a single logical repository-wide head for every worker. A physical writer may serialize short commits; unrelated workspace generations remain logically independent.

First correctness implementation: use a local SQLite transactional authority holding canonical changesets, small blobs, workspace/target roots, operations, pins and request outcomes. This minimizes cross-file durability ordering while building the reference model. It is **not a final performance choice**: compare one append-only object/root journal and immutable object segments with separate metadata after real crash and workload tests. Large blobs can move out only with an explicit staging/GC protocol.

Local mutation sequence:

    prepare declared input IDs and source changes
    begin short durable transaction
      check expected workspace/target generations
      check request identity/epoch and canonical payload
      write required canonical closure, or check durable staged blobs
      select updated mutable root(s)
      record pin changes, operation, retry outcome, event intent
    commit durably and acknowledge

Changesets are immutable and may be created without advancing public refs. In a split object+metadata store, a durable staging root/epoch must protect objects from GC **before** an authoritative target references them. Never acknowledge a root referencing missing blobs. Reader epochs/leases and staging pins protect compaction and repair. Deleting a cache is safe; deleting the root authority is not.

Backup must include a consistent root/operation state and all reachable canonical blobs/changesets. Private recoveries expire by time/count/bytes; named checkpoints and accepted history remain protected. An immortal operation chain must not accidentally retain every private capture. Only root transitions that truly require coordination use compare-and-swap.

Full-source data integrity uses versioned domain-separated hashing, canonical length limits, checksums/framing on physical records and verification at untrusted boundaries. Cryptographic content hashes do not establish access rights, authenticity of actors, or semantic correctness.

## 7. Machine API, remote clients and syncing

Core APIs accept exact immutable ChangesetIds or explicit resource-generation preconditions:

- Read file/range/tree, compare trees, list history, blame/path queries, inspect conflicts.
- Create a changeset from path edits, capture/recover, construct merges, inspect operation history.
- Synchronize selected immutable roots, propose a candidate, get validation status, conditionally advance a target.

CLI, library and service expose the **same semantics**, but not necessarily the same physical execution. A local full repository works entirely offline; thin clients can fetch only requested objects and ask a service to merge or diff. Missing promised objects are not silently treated as deletions. RPCs are bounded, batchable, resumable and return structured errors and immutable pagination cursors.

Large deployments may use stateless compute and independently cached durable objects; node/regional cache tiers and sparse/lazy workspaces are optional. Separate durable storage from serving capacity and avoid requiring full repositories on every CI runner. Protect object retrieval authorization, including directory-scoped restrictions and cross-tenant existence leaks; hashes alone are not authorization. Failure isolation matters as much as cache hit rate.

Request idempotency keys have authenticated actor + authority epoch scope. Replay returns the original durable outcome while valid; retired epochs are rejected rather than silently executing delayed requests. Cancellation and network timeouts do not revoke already committed state.

## 8. Review, validation and publication

Four distinct boundaries: **private capture → optional restricted draft sync → proposal/review → accepted target transition**. Do not send every private agent loop to a normal Git remote and promise quiet events; a third-party host may trigger webhooks and CI for pushed refs.

A candidate names exact revisions, computed merge result, base/target generation, algorithm, validation recipe and policy epoch. Tests and human approvals are evidence **about** that exact candidate; do not include future evidence in the candidate's own digest. Validation runs on the immutable result or an isolated exact materialization, not a live workspace mutable during the test.

Protected landing checks current target, policy/authority and qualifying evidence and atomically updates **only the target and its receipt/outbox**. Changed target or policy may require recomposition and revalidation. An organization may instead maintain separately accepted and verified targets, or choose post-merge/pre-deployment checks, but must state the weaker guarantee. No design eliminates necessary ordering on one shared linear target.

If the destination is GitHub/GitLab/etc., that host controls ref, review and CI policy. A local AVC transaction cannot promise cross-host atomicity; use actual remote protections/merge queues and reconcile unknown network results.

## 9. Git import/export, platform and trust boundary

Existing Git repositories are adoption inputs; Git is **not** canonical AVC source truth. Preserve original Git objects/IDs when unchanged and keep mapping to native changesets. Native rewrites/export will generally produce new Git commits and lose some private metadata; state this explicitly. Do not silently share an index/worktree with a Git writer. Export unresolved conflicts only through a capable explicit transport, not as a resolved Git commit.

Test practical compatibility: paths/Unicode/casefolding, executable bits, symlinks, ignore rules, attributes/EOL/filters, tags/signatures, Git LFS, submodules and partial/shallow histories. Unsupported operations fail before destructive mutation. Binary source is first-class opaque data; no mandatory parser or AST format.

Untrusted repository data and configuration do not implicitly execute hooks, plugins or language tooling during clone, fetch, status or diff. Process execution belongs to an explicitly authorized runner. Agent provenance and optional read sets are useful advisory/validation inputs, not a required source identity or proof of semantic independence.

## 10. Decisions, open bets and proof of value

**Decided as architectural requirements:** native model; offline full repository; per-workspace/target conditional roots; immutable canonical source objects; no-checkout APIs; bounded private recovery; conservative typed conflicts; deliberate publication; explicit Git exchange; safety gates before real user trust.

**Leading reference:** parent-relative canonical changesets plus eagerly/lazily derived persistent tree views. **Not yet format-stable.**

**Serious alternatives:** canonical snapshot-root revisions with changed-path hints, and Pijul/Darcs-style causal patch graphs where selective undo and change commutation have real value. These are not optional cosmetic tweaks; a winning alternative would replace the canonical history model.

**Open implementation choices:** tree-node layout (flat vs deterministic radix/prolly), physical store and durability protocol, hash/codec, large-file chunking/deltas, subtree move/copy semantics, native protocol cache topology, merge driver selection, initial Git compatibility subset and naming.

The comparison must include end-to-end creation + first-read + history query + merge + recovery + GC + sync, not just cheap append. Data loss, incorrect unreported automatic merging, or false protection claims are release blockers. The current native implementation does not exist; the research fixtures are counterexamples, not performance proof.
