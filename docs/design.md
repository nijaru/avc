# AVC architecture: source state first

**Re-evaluated:** 2026-10-14  
**Status:** concrete native *reference design*, not implemented, benchmarked, or format-frozen  
**Evidence:** [research](research.md) · **Delivery gates:** [roadmap](roadmap.md)

This document supersedes the previous assumption that parent-relative changesets should be canonical. The goal is to make the best general-purpose source-control system for ordinary development **and** rapidly growing agent workflows. Git, Jujutsu, Scion, Mononoke, Pijul and Enroute are sources of evidence, not ancestors whose formats or algorithms must be reproduced.

## 1. Derive the core from user-visible contracts

A VCS is first responsible for preserving, identifying, retrieving and combining **exact source states**. The work is done by people, agents, scripts and editors, not necessarily through a controlled edit API. It can include malformed text, binaries, directory moves, symlinks and files produced by arbitrary processes.

Representative environments:

| Location | Meaningful operation | Binding requirement |
| --- | --- | --- |
| Laptop, desktop, persistent VPS | Normal editor, offline source history and recovery | Source, history and undo can work without network or daemon |
| SSH/cloud developer environment | Move/reopen work on another machine | Durable independent state; separate filesystem materialization |
| Short-lived container or microVM | Inspect, edit, test, submit, destroy | Share immutable source/cache; acknowledge handoff before destruction |
| CI/build job | Read exact source and produce a verifiable artifact | No unnecessary history clone; isolated exact materialization if needed |
| Source/query service | History/blame/diff without working copy | Immutable query API over exact IDs |
| Large team/agent fleet | Many private attempts, few deliberately accepted results | Cheap independent writes; narrow shared-target coordination |
| Production runtime | Deploy a pinned artifact or source tree | Link provenance; do not require running the VCS server |

Separate **source**, **mutable workspace**, **private recovery**, **shared publication**, and **execution/deployment artifacts**. Do not put processes, prompts, model internals, build caches or VM filesystem images into canonical source history.

**Native is a product decision.** Git format, object hashes, on-disk files, refs, pack format, interoperability and even the word "commit" are **not requirements**. A future adapter is allowed only on its own merits. The initial system must offer a self-contained, inspectable native export and backup before becoming trusted.

## 2. Canonical authority: the exact source snapshot

**Selected reference:** each immutable revision commits to the exact resulting **StateId** (root of a persistent Merkle namespace) and explicit ancestry. A change manifest, path diff, copy/move hint or merge recipe may accompany the revision, but is *not an independently mutable source authority*.

Why this is the strongest starting position:

1. **Arbitrary file writers.** Existing files and resulting bytes are observable; the author's unique sequence of causal editing operations often is not.
2. **Most meaningful states are consumed.** Humans, CI and agents frequently inspect/test/merge the source they record. State roots avoid requiring reconstruction from an arbitrarily long predecessor chain before such use.
3. **Immutable structural sharing.** Editing a few paths changes a bounded region of a persistent namespace, rather than physically duplicating a repository. Copying or moving an unchanged subtree can reuse one immutable subtree reference.
4. **One source commitment.** The root identifies exactly what a build, remote reader, reviewer or integration result is about, without relying on an optional derived-data service.
5. **Thin-client and offline symmetry.** The same root supports local operations, sparse requests and full replication. It is not necessary for every agent to store the entire history.

This is a logical claim, **not** proof of faster commits than changeset-first storage. Canonical snapshots require choosing and maintaining a versioned tree encoding. A very large flat directory could cause write amplification. See the qualification plan, and revisit this choice if the complete workload measurements favor another model.

### Logical types

Illustrative contracts only; not final Rust types or wire serialization:

    State  = content-addressed typed namespace root

    Entry  = Directory { map_root }
           | File { content_id, executable, kind }
           | Symlink { target_bytes }
           | Conflict { terms_id }

    Revision = {
        format_version,
        state: StateId,
        parents: Vec<RevisionId>,      // ancestry, not a recipe for file bytes
        change: LogicalChangeId,      // identity across amendments, not algebra
        author, description, metadata
    }

    Evolution = { prior: Vec<RevisionId>, next: Vec<RevisionId>, operation }
    WorkspaceRoot = { workspace_id, generation, revision, materialization }
    TargetRoot    = { name, generation, revision, policy_version }
    Operation     = { scoped_request_id, affected_roots, before, after, outcome }

**StateId** commits only to the exact tracked logical source namespace and entry contents/modes, not who created it. Equal source in the *same canonical format* produces the same StateId even if ancestry differs. **RevisionId** commits to that state, ancestry, logical change identity and canonical revision metadata. Two revisions can therefore share StateId while remaining distinct history. Histories may have multiple parents without giving parents any implicit ability to alter the selected State.

**LogicalChangeId** is a stable label for related authored versions, useful for amend/rebase, not an assertion that a patch commutes or can be cleanly reapplied. Two divergent descendants of the same change remain distinct revisions; nothing selects a "latest" version by timestamp. History split/fold has explicit evolution records. Revision and state IDs are content-addressed; roots like workspace heads and targets are the only mutable pointers.

**Source namespace:** path components are exact byte strings with published validation and normalization *policy*. In v0 disallow zero-length, dot, dot-dot, slash and NUL components; never silently fold case or normalize names when importing/materializing. Support regular/executable file contents, symlink target bytes, typed unresolved conflicts and explicit empty directories. Devices, hardlink identity, platform-specific ACLs, extended attributes and line-ending transforms require a separate supported-contract decision; never silently drop a tracked feature.

**Canonical encoding caveat:** changing the representation of a canonical Merkle directory can change StateIds. Do not pretend that derived-index versioning makes *canonical tree format migration free*. Pin format/hash vectors before stable release; preserve old decoders and provide explicit migration/object mapping for a future upgrade. Physical compression, page cache and object placement are independent of canonical IDs, but the bytes of a canonical tree node are not.

## 3. Editing source: operations are APIs, not mandatory history truth

Workspaces may produce changes by ordinary filesystem scans, a VCS-aware editor, or a headless source API. The core accepts immutable-state transformations with exact preconditions:

    apply(base_state, operations, expected_entries) -> Result<StateId>

    Put(path, entry, expected_previous)
    Delete(path, expected_previous)
    CopySubtree(source_state, source_path, destination, expected_destination)
    MoveSubtree(source_path, destination, expected_source, expected_destination)

These operations are **recipes used to construct** a new persistent tree. They are not obligatory canonical revision patches. Every operation must validate its input StateId, path types, expected subtree/file IDs, destination collision and any overlapping edits; one request either constructs a complete immutable new source state or fails without publishing a partial state.

A CopySubtree references a **subtree object**, not necessarily an entire historical revision. A move updates namespace bindings while retaining that subtree object; it must not enumerate all unchanged descendant files. This avoids the earlier proposal's unnecessary durable Graft-from-old-revision dependency. A copy across repositories needs the exact source subtree and descendant object closure.

When a normal editor renames a directory but AVC sees only before/after byte states, the source result is still exact. Attribution of rename **intent** is optional and cannot be asserted merely from identical content/inodes. Verified provenance may be recorded with a revision for review/blame and can be re-derived heuristically if absent; it cannot override the committed StateId.

A small changed-path manifest or delta may be stored with the revision to speed path history or network transport, but must be verified against the parent and resulting State. Do not let the "manifest says X" view diverge from the "tree says Y" view. It is safe to delete/rebuild acceleration metadata.

### The source-write loop

1. Observe exact changed files or receive explicit expected-object edits.
2. Protect/capture input bytes as required for the operation.
3. Build new immutable content and affected tree nodes; reuse all unchanged subtrees.
4. Construct an immutable Revision referencing the resulting StateId and exact parents.
5. In a short transaction conditionally move **only the affected workspace root**, recording the prior value and durable retry outcome.
6. Report source state separately from any incomplete filesystem materialization.

No shared repository/global-head CAS is required when two workers write different workspaces. Physical short-write serialization in a local DB is not the same as *logical* conflict.

## 4. Three different time scales: recovery, authored history and publication

Treat them separately instead of forcing one commit object to handle all of them.

**Private recovery:** bounded, best-effort captures of admitted local file bytes under explicit coverage/retention. Very frequent agent edit/test cycles can use a compact overlay or file-delta journal plus pinned base StateId and materialize a full State only when needed. Those captures are a **different lifecycle class**, not immortal revisions or public changes. Their reconstruction is deterministic against pinned inputs, and retention/GC removes expired captures only when no stronger pin exists.

**Authored history:** immutable source snapshots selected by the developer/agent as meaningful revisions; stable change identities and explicit rewrites. These are the objects that should be directly queryable, tested, reviewed and merged.

**Publication:** a candidate produced from exact revisions and a target base is validated under an explicit policy, then accepted by a target-specific conditional transaction. Some large fleets may choose *accepted* and *verified* target separation; that is a weaker, separately named guarantee, not an assertion that pre-merge tests can scale infinitely.

Do not automatically publish private captures or trigger CI/webhooks on each edit. Protect admitted bytes of ephemeral workers durably before destroying their VM/container.

## 5. Merge and conflict behavior

A merge algorithm consumes exact base/left/right States and ancestry and returns an immutable resulting State (possibly with typed conflict entries). It does not rely on commits being changeset-first, and it does **not** automatically know that disjoint paths are semantically independent.

Reference source merge:

    identical sides   -> select common entry
    one side == base  -> select the other
    both directories  -> recursively merge children
    otherwise         -> deterministic versioned file/namespace merge
                         or retain a typed conflict

Handle file-vs-directory, symlinks, modes, rename/edit, create/create, delete/modify, binaries, multiple merge bases and conflict terms explicitly. Use an explicit conflict-preserving virtual-base or n-way procedure rather than picking an arbitrary ancestor. A merge candidate records the exact algorithm/version/resolution inputs. No automatic merge is proof that the combined code works.

A Conflict contains base, alternatives and precise source provenance, **not merely marker bytes**. A conflicted State is valid VCS history but cannot be silently represented as a normal resolved file or accepted by a policy prohibiting conflicts. Conflict resolution creates a new State/Revision with the exact solution. Exact-context replay of a past resolution requires verified matching conditions. Syntax-aware and LLM-assisted drivers are optional proposed resolutions; they cannot mutate source invisibly.

**Undo** is a resource operation restoring a previous private head if expected generations still match; **revert** is a new content change and may conflict. True causal selective undo of identical independent edits is a separate algebraic property that this reference model does not promise. Rebase/amend creates immutable successor revisions; originals remain recoverable under retention. An intentionally reverted change must remain eligible for explicit reapplication; no eternal "seen ChangeId" filter.

## 6. Working directories and real filesystem safety

Every independently writing human/agent session should have a distinct writable workspace or managed virtual namespace; immutable State/Revision objects, readonly caches and verified build outputs may be shared. A workspace is not a security sandbox and is not identical to one authenticated actor or agent process.

A filesystem watcher is a hint, not a committed source observation. File mtimes/inodes can be stale or reused. Scanning files while editors write does not produce an atomic multi-file snapshot; annotate capture coverage honestly. Unsaved editor buffers are not protected by filesystem inspection.

In a workspace with uncontrolled OS writers, a portable check-then-overwrite does not provide guaranteed no-clobber. **The safe default for source restoration is a new directory.** An in-place switch must preserve before-images, record materialization intent and require real writer exclusion/coordination for stronger claims. A repository transaction and a filesystem checkout are independently recoverable steps.

Full repositories work offline. Sparse/lazy clients carry explicit promised-object state and must report a missing object as missing; they may not treat it as an absent path. Copy-on-write clones, reflinks, overlayfs, FUSE/virtual filesystem, and microVM snapshots can accelerate materialization, but are not mandatory components.

## 7. Derived indexes and tree data structures

Canonical persistent Merkle namespace objects are **source truth**. Other structures are derived and rebuildable:

- Revision-graph generations, skip links and selected ancestry/reachability indexes.
- Changed-path summaries, postings, negative Bloom filters, path/copy history.
- Blame and line-origin tables keyed by source and algorithm version.
- Optional symbol/AST indexes, search and change observation metadata.
- Physical fetch/object-set and cross-revision transfer indexes.

The initial reference tree may use sorted flat directory maps for correctness and realistic small-directory performance. **Before freezing the format**, compare deterministic bounded sharded/radix directory pages and ordered POS/prolly pages on huge/wide directories and rename/copy/diff workloads. Root hashing is defined by the chosen canonical format; caches may build independent query representations. A very wide unpaged directory could rewrite an entire directory blob per tiny edit, so do not mistake a toy flat directory tree for the final scalability solution.

Tree equality (matching canonical StateId) is fast. Single path read visits directory nodes and blob. Diff prunes identical subtrees. Copy/move of an unchanged subtree relinks immutable references. Text-level merge, detecting renames under ambiguous changes and validating combined program behavior still cost real work.

## 8. Storage authority and fault-tolerant transactions

The logical model **does not select a physical object store or a distributed consensus protocol**.

First implementation: one local durable SQLite transaction authority for canonical small objects, workspace roots, operation log, staging/pins and retry outcomes. It is a correctness baseline, not a permanent mandate. Store ordinary blobs directly initially; evaluate immutable segment files for large content or throughput when verified stage/flush-before-root and GC protocols exist. Set explicit WAL durability settings, handle full disk and long readers, and test crash stages.

Other candidates: append-only object+root journal or immutable compacted object segments with separately transactional root authority. Each must uphold the same guarantees: acknowledged roots have complete durable canonical closure; no in-flight object is collected; readers/backup see a coherent point; orphaned staged objects can be reclaimed safely. Domain-separated cryptographic content IDs validate bytes, not author or permission.

Physical placement, compression, delta coding, local numeric indexes and packed bundles **must not alter canonical source/revision identity**. Enroute's commit segments and dense IDs demonstrate one Git-specific physical optimization, not an AVC requirement. A locally assigned physical index number may be useful, but a globally coordinated sequential number cannot be necessary to create a source object offline.

Retention classes include expiring captures, named pins, independent draft roots, active readers/transactions, accepted history, and backups. GC preserves their reachable canonical objects. Deleted cached indexes must be reconstructible; deleted authoritative objects cannot.

## 9. Native source API and synchronization

Define the same semantic calls for local library, CLI and optional hosted service, independent of whether workspaces exist on the caller:

    read_file(StateId, path, range)
    list_tree(StateId, path)
    diff(StateId, StateId)
    apply_edits(StateId, preconditioned_ops) -> StateId
    record_revision(StateId, parents, LogicalChangeId, metadata) -> RevisionId
    merge(base, left, right, algorithm_id) -> candidate StateId
    advance_workspace(id, expected_generation, revision)
    propose(target, exact_revision_and_state, policy_id)
    land(target, expected_generation, candidate_id, evidence)

APIs return bounded results, exact identifiers, typed errors, and retry/epoch-scoped idempotency. Public acceptance checks live authorization, target generation, policy and exact evidence in one authority transaction. Tests run outside locks. A host cannot retroactively claim test evidence for a different state or toolchain.

Transport supports **both** sparse/requested object hydration and bounded, compressed, resumable bulk transfer of a selected State/Revision closure. Which objects to batch or cache is determined by workload. A cold clone should not incur one RTT per file, nor should a sparse reader download the entire repository. Shared immutable caches can serve many short-lived workers, but require authorization, backpressure and outage isolation.

Source ID, revision ID and target authority may move between local, VPS, CI and service environments without changing meaning. Deployment artifacts carry exact source provenance; they are not a revision subtype. Native export provides source and metadata closure independent of any Git format.

## 10. Decisions and reversal conditions

**Firm design requirements:** native/non-Git source format; local-first and optional thin client; canonical exact source; separate recovery/history/publication lifetimes; independent workspace roots; conservative conflicts; safe restore; native object queries/sync; explicit security, retention and backup contracts.

**Selected reference, awaiting implementation evidence:** source-root snapshots in immutable Revision DAG; stable LogicalChangeId/evolution metadata; typed Merkle directory namespace; one local transactional authority; no mandatory causal patch graph.

**Open decisions:** canonical directory page layout and its migration story; source metadata/mode/path semantics beyond v0; when to persist changed-path acceleration; local object segmentation/chunking and compression; server cache/transfer trade-offs; change-graph alternative where causal selective undo yields major benefit; snapshot-first versus changeset-first full-workload comparison before irreversible format stability.

**Explicitly out of scope:** native Git interchange requirement, Git commit/tree/pack compatibility, Pijul-style patch algebra as a default, mandatory Raft, forced cloud connectivity, an agent orchestrator, and a parallel VM/filesystem-versioning product.

The old Rust source still implements a Git-command wrapper, not this model. No unrun benchmark is used to assert that AVC already outperforms Git, Jujutsu, Scion or any competitor.
