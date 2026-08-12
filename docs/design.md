# AVC: Atomic Version Control

**Architecture and implementation plan**
**Status:** Implementation blueprint; evolve it as the working tool grows
**Research current through:** 2026-08-12
**Primary implementation language:** Rust
**Target:** New software projects; human-first and automation-safe; no Git-compatible core required

---

## Contents

- [0. How to use this document](#0-how-to-use-this-document)
- [1. Executive design](#1-executive-design)
- [2. Goals, non-goals, and design principles](#2-goals-non-goals-and-design-principles)
- [3. User model and terminology](#3-user-model-and-terminology)
- [4. System architecture](#4-system-architecture)
- [5. Identity and canonical encoding](#5-identity-and-canonical-encoding)
- [6. Canonical object model](#6-canonical-object-model)
- [7. Path and filesystem model](#7-path-and-filesystem-model)
- [8. Append-only storage engine](#8-append-only-storage-engine)
- [9. File-content strategy](#9-file-content-strategy)
- [10. Working-copy engine and automatic savepoints](#10-working-copy-engine-and-automatic-savepoints)
- [11. Change and history semantics](#11-change-and-history-semantics)
- [12. Merge and conflict design](#12-merge-and-conflict-design)
- [13. Workspaces and materialization](#13-workspaces-and-materialization)
- [14. Synchronization and server design](#14-synchronization-and-server-design)
- [15. Build outputs, generated files, and artifacts](#15-build-outputs-generated-files-and-artifacts)
- [16. CLI and machine API](#16-cli-and-machine-api)
- [17. Security and trust model](#17-security-and-trust-model)
- [18. Correctness invariants](#18-correctness-invariants)
- [19. Verification and testing strategy](#19-verification-and-testing-strategy)
- [20. Performance model and proof obligations](#20-performance-model-and-proof-obligations)
- [21. Rust implementation architecture](#21-rust-implementation-architecture)
- [22. Build roadmap and acceptance criteria](#22-build-roadmap-and-acceptance-criteria)
- [23. Suggested initial pull-request sequence](#23-suggested-initial-pull-request-sequence)
- [24. Open architecture decisions](#24-open-architecture-decisions)
- [25. Risks and scope controls](#25-risks-and-scope-controls)
- [26. Features explicitly deferred](#26-features-explicitly-deferred)
- [27. Immediate implementation prompt](#27-immediate-implementation-prompt)
- [28. Annotated primary sources](#28-annotated-primary-sources)
- [29. Final recommendation](#29-final-recommendation)

---

## 0. How to use this document

This document defines the intended working tool and an incremental path to build it. The product model and correctness goals are durable direction. Storage formats, algorithms, and subsystem boundaries may evolve as implementation reveals better choices.

Normative terms describe the intended mature design; milestones implement them incrementally:

- **MUST**: required for correctness or for the product model.
- **SHOULD**: strong default; deviation requires an architecture decision record (ADR).
- **MAY**: optional or deferred.

Evidence labels:

- **[FACT]**: documented behavior or architecture from a primary source.
- **[MEASURED]**: a published measurement; it applies only to the measured system and workload.
- **[DECISION]**: a proposed AVC design decision.
- **[OPEN]**: a decision that requires a prototype, benchmark, or security review before the format is frozen.

### Instruction to the implementing session

Build one working end-to-end milestone at a time and keep each milestone usable. Start with an AVC local loop—`init`, `checkpoint`, `status`, `log`, and `restore`—on immutable logical objects and durable local state. Do not begin with a daemon, custom filesystem, peer-to-peer replication, semantic/LLM merging, a full forge, or Git round-tripping.

The production-quality local tool will include:

1. immutable snapshots;
2. stable change identities;
3. an operation log with undo/restore;
4. multiple workspaces;
5. deterministic merge with first-class conflicts; and
6. bounded automatic savepoints.

The second milestone is AVC draft synchronization and atomic publication.

---

## 1. Executive design

### 1.1 Product definition

> **AVC (Atomic Version Control) continuously protects work, organizes it into stable logical changes, makes repository operations reversible, and publishes shared history atomically.**

AVC supports interactive development, large monorepos, automated dependency updates, generated code, continuous refactoring, CI, coding agents, and many concurrent contributors.

### 1.2 Central design thesis

AVC separates three forms of history that Git commonly collapses into commits and refs:

```text
private recovery history       logical development history       shared project history
savepoints                ->   changes and revisions        ->   publications
```

- A **savepoint** protects local work. It is automatic, private by default, compactable, and not a collaboration event.
- A **change** is a stable unit of intent. It can be amended, split, folded, reordered, or rebased without losing identity.
- A **revision** is one immutable version of a change.
- A **publication** is an intentional atomic transition of shared project state. It is the normal trigger for review, indexing, webhooks, and CI.

This separation is the primary UX and systems-level improvement. Fast commit creation alone is not a sufficient product.

### 1.3 Recommended architecture

**[DECISION] AVC SHOULD be a userspace multiversion object database with an ordinary filesystem working copy.**

It SHOULD use:

- immutable content-addressed objects;
- persistent Merkle directory maps;
- a separate repository operation graph;
- a single append-only physical write path;
- derived, rebuildable indexes;
- stable change IDs distinct from revision IDs;
- conflict-bearing revisions;
- first-class workspaces;
- quiet draft synchronization; and
- atomic compare-and-swap publication.

It SHOULD NOT require a custom filesystem. A lazy virtual working copy can be added after the object and workspace interfaces are stable.

### 1.4 What is state of the art here

No existing VCS is state of the art on every relevant metric. AVC should combine the strongest demonstrated ideas by metric:

| Metric | Strong public precedent | AVC use |
|---|---|---|
| Recoverable history editing and stable change identity | [Jujutsu](https://jj-vcs.github.io/jj/latest/tutorial/) | Stable `ChangeId`, working-copy revisions, operation log, undo/restore, conflict-bearing revisions |
| Very large repository and lazy-working-copy scale | [Sapling scale architecture](https://sapling-scm.com/docs/scale/overview/) and [EdenFS](https://github.com/facebook/sapling/blob/main/eden/fs/docs/Overview.md) | Incremental working-copy state, on-demand content, sparse materialization, derived graph indexes |
| Reusable multiversion maps | [ForkBase/POS-Tree, PVLDB 2018](https://www.vldb.org/pvldb/vol11/p1137-wang.pdf) and Dolt’s [prolly-tree design](https://www.dolthub.com/blog/2022-06-27-prolly-chunker/) | Deterministic persistent maps for large directories and chunk manifests |
| Avoiding log-on-log write amplification | [XLL, NSDI 2026](https://www.usenix.org/conference/nsdi26/presentation/shawger) | One append path for object payloads and transaction durability |
| Fine-grained change hints | [SolFS, USENIX ATC 2025](https://www.usenix.org/conference/atc25/presentation/pan) and [SkySync, FAST 2026](https://www.usenix.org/conference/fast26/presentation/zhang-zhihao) | Optional write-range/checksum hints; never the correctness source |
| Large-file deduplication | [VectorCDC, FAST 2025](https://www.usenix.org/conference/fast25/presentation/udayashankar) | Selective CDC for large files, not ordinary source files |
| Structured merge | [Mergiraf architecture](https://mergiraf.org/architecture.html) | Line merge first; optional deterministic syntax-aware driver; safe fallback |
| Storage correctness testing | [Metis, FAST 2024](https://www.usenix.org/conference/fast24/presentation/liu-yifei) | Reference model, state exploration, crash injection, differential checking |
| Build-result separation | [Bazel remote caching](https://bazel.build/remote/caching) | Separate action cache and artifact CAS from source history |

The cited measurements are evidence that the techniques can matter, not performance predictions for AVC. For example, XLL reports 5.5× higher write throughput and 73% less write amplification in its TiKV prototype; SolFS and SkySync report large reductions for their synchronization workloads; VectorCDC reports 21–46× higher chunking throughput than prior vectorized techniques. AVC must reproduce benefits on source-control workloads before committing to comparable mechanisms.

---

## 2. Goals, non-goals, and design principles

### 2.1 Goals

AVC MUST provide:

1. **A small mental model.** Workspace, savepoint, change, revision, operation, and publication have distinct meanings.
2. **Automatic recovery.** Ordinary edits are protected without manual commits, staging, or stashes.
3. **Fast common operations.** Status, diff, snapshot, log, workspace switching, merge, and sync scale with changed or requested data wherever possible.
4. **Safe history editing.** Split, fold, reorder, rebase, and restore are normal reversible operations.
5. **Stable logical identity.** A change retains identity while its immutable revision ID changes.
6. **First-class conflicts.** A merge or rebase may succeed while producing a revision containing unresolved conflict data.
7. **Efficient collaboration.** Backing up unfinished work does not automatically create public branches, notifications, webhooks, review updates, or full CI runs.
8. **Structured automation.** IDEs, bots, build systems, and agents use a typed API with atomic operations and structured errors.
9. **Crash safety.** A visible transaction is complete; an incomplete transaction is invisible.
10. **Format evolution.** Hash algorithms, physical compression, indexes, and transports can change without redefining logical history.

### 2.2 Non-goals for the first implementation

AVC v1 is not:

- a Git storage backend or a Git-compatible protocol;
- a complete GitHub/GitLab replacement;
- a package registry, issue tracker, code-search service, or CI product;
- a kernel filesystem;
- a CRDT text editor;
- a peer-to-peer network;
- a semantic source-code database;
- an LLM conflict resolver;
- a global cross-tenant deduplication service; or
- a system that promises byte-for-byte preservation of every filesystem metadata feature.

One-time Git import and snapshot/release export MAY be added later, but Git concepts MUST NOT constrain AVC's object model.

### 2.3 Design principles

#### Snapshot truth, operation intent

The immutable snapshot graph is authoritative for source state. The operation graph records how repository views changed and enables undo, audit, and recovery. Operation intent MAY improve UX and merge behavior, but replaying an imperative edit log MUST NOT be the only way to reconstruct a revision.

#### Logical format separate from physical layout

Object identity MUST depend on canonical logical bytes, not segment placement, compression, delta chains, database pages, cache contents, or insertion order.

#### Derived indexes are disposable

Deleting any cache or derived index MUST affect performance only. Repository correctness MUST be recoverable from immutable objects, committed transaction records, and root manifests.

#### Watchers are hints

Filesystem notifications, editor events, storage checksums, and write ranges MAY reduce scanning and hashing. They MUST NOT be trusted as complete. Overflow or uncertainty causes reconciliation.

#### Publication is a transaction

Shared project state changes through an atomic manifest with an expected prior view. Uploading objects and announcing project state are separate operations.

#### One command, one concept

The CLI SHOULD avoid overloaded commands analogous to Git’s multiple `reset`, `checkout`, and `restore` modes. Destructive-looking operations MUST be recoverable through the operation log.

#### Complexity must be earned

Prolly trees, CDC, set reconciliation, virtual filesystems, and structured merge are adopted only when measurements show a material advantage over simpler designs.

---

## 3. User model and terminology

### 3.1 Workspace

A **workspace** is a checked-out or partially materialized filesystem view plus its local state. One repository can have multiple workspaces. A workspace is not automatically a public branch.

Each workspace has:

- a stable `WorkspaceId`;
- its current editable revision;
- its current operation head;
- a materialization profile;
- a working-copy state cache;
- optional draft-sync state; and
- an owner/lease record.

### 3.2 Savepoint

A **savepoint** is an automatically or explicitly captured revision of the current editable change.

Automatic savepoints:

- are local by default;
- do not add another semantic parent revision;
- keep the same `ChangeId` and semantic parents as the revision they replace;
- produce a new immutable `RevisionId` only when the tree or revision metadata changed;
- are retained according to local policy; and
- do not trigger public events.

An explicit named checkpoint pins a recoverable repository view until the user removes it.

### 3.3 Change

A **change** is a logical unit of work identified by a stable random `ChangeId`. It can have many immutable revisions over time.

Examples:

- “Add streaming parser”
- “Rename configuration fields”
- “Fix allocator race”

Amending or rebasing a change changes its `RevisionId`, not its `ChangeId`.

### 3.4 Revision

A **revision** is an immutable snapshot containing:

- a root directory ID;
- zero or more semantic parent revision IDs;
- a stable change ID;
- description and author metadata; and
- optional unresolved conflict objects reachable from the tree.

A merge revision has two or more parents. Editing an existing change normally produces a replacement revision with the same change ID.

### 3.5 Operation

An **operation** is an immutable transition to a complete repository **view**. Examples include:

- capturing a savepoint;
- creating a change;
- splitting or folding changes;
- rebasing;
- merging;
- switching a workspace;
- updating a public name;
- restoring an earlier view; and
- accepting a synchronized remote view.

Undo appends a new operation that restores or reverses an earlier view. It does not erase the operation being undone.

### 3.6 Publication

A **publication** is an atomic shared update. It includes the expected previous public view, the proposed new public view, selected revisions and names, policy metadata, and an idempotency key. A host accepts the whole update or rejects it.

---

## 4. System architecture

```text
┌──────────────────────────────────────────────────────────────────┐
│ Human CLI / IDE / typed automation API                           │
└──────────────────────────────┬───────────────────────────────────┘
                               │ repository transactions
                               ▼
┌──────────────────────────────────────────────────────────────────┐
│ Repository engine                                                │
│                                                                  │
│ change/revision graph   operation/view graph   merge engine       │
│ workspaces              retention policy       query layer        │
└───────────────────┬────────────────────┬─────────────────────────┘
                    │                    │
             verified snapshots          │ immutable queries
                    ▼                    ▼
┌──────────────────────────────┐  ┌───────────────────────────────┐
│ Working-copy engine          │  │ Derived indexes              │
│                              │  │                               │
│ watcher hints                │  │ object locations             │
│ dirstate                     │  │ revision graph/generations    │
│ scan and checkout            │  │ change-id lookup             │
│ conflict materialization     │  │ path history and changed paths│
└───────────────┬──────────────┘  └───────────────┬───────────────┘
                │                                  │ rebuildable
                ▼                                  ▼
┌──────────────────────────────────────────────────────────────────┐
│ Append-only local object store                                   │
│                                                                  │
│ active segment -> sealed immutable segments -> compaction        │
│ canonical objects + operation records + transaction commits      │
└──────────────────────────────┬───────────────────────────────────┘
                               │ AVC bundles
                               ▼
┌──────────────────────────────────────────────────────────────────┐
│ Remote object service                                            │
│                                                                  │
│ quarantine segments  draft roots  atomic public views            │
│ authorization         quotas       event/policy hooks             │
└──────────────────────────────┬───────────────────────────────────┘
                               │ explicit transitions only
                               ▼
┌──────────────────────────────────────────────────────────────────┐
│ Review / CI / indexing / notification services                   │
└──────────────────────────────────────────────────────────────────┘
```

### 4.1 Local process model

**[DECISION] The MVP MUST serialize repository writes with an exclusive repository transaction lock.** Immutable reads MAY run without that lock against a captured root manifest.

An optional `avcd` daemon can later own:

- filesystem watching;
- debounced savepoints;
- background indexing;
- compaction;
- remote draft synchronization; and
- a local typed RPC endpoint.

The operation format SHOULD remain DAG-capable so concurrent or remotely divergent operations can be represented later, but lock-free multiwriter support is not an MVP requirement. Jujutsu demonstrates an operation/view DAG for concurrency and recovery; AVC can retain the model without taking its full concurrency implementation immediately ([Jujutsu concurrency design](https://jj-vcs.github.io/jj/latest/technical/concurrency/)).

### 4.2 Repository directory

Proposed on-disk layout:

```text
.avc/
├── config.toml                 # repository policy; not content-addressed
├── state/
│   ├── heads                   # atomic current operation-head set
│   ├── store-manifest          # active/sealed segment generations
│   └── format                  # repository format and feature gates
├── store/
│   ├── active-000001.seg       # current append target
│   ├── segments/
│   │   ├── 00000001.seg
│   │   └── 00000002.seg
│   └── quarantine/             # incomplete imports/sync transactions
├── cache/
│   ├── object-index
│   ├── revision-graph
│   ├── change-index
│   └── path-index
├── workspaces/
│   └── <workspace-id>/
│       ├── state
│       ├── dirstate
│       └── conflicts
├── leases/
├── locks/
└── tmp/
```

Everything under `cache/` MUST be deletable and rebuildable. Temporary and quarantine data MUST never be considered reachable until a committed root references it.

---

## 5. Identity and canonical encoding

### 5.1 ID classes

AVC uses two categories of identifiers.

#### Content-derived IDs

These identify immutable objects:

- `BlobId`
- `ChunkMapId`
- `FileManifestId`
- `DirectoryId`
- `ConflictId`
- `RevisionId`
- `ViewId`
- `OperationId`
- `PublicationId`

Conceptually:

```rust
struct ObjectId {
    hash_algorithm: HashAlgorithm,
    digest: [u8; 32],
}
```

The wire/text form MUST be self-describing and MUST NOT assume one permanent hash algorithm or digest length. Git’s SHA-256 transition illustrates the cost of ecosystem-wide fixed-hash assumptions ([Git hash transition design](https://git-scm.com/docs/hash-function-transition)); multihash provides a relevant self-describing precedent ([Multihash](https://multiformats.io/multihash/)).

#### Stable random IDs

These identify logical entities whose contents evolve:

- `ChangeId`
- `WorkspaceId`
- `RepositoryId`
- client idempotency tokens

Use cryptographically secure random IDs with at least 128 bits of entropy. Do not derive `ChangeId` from revision contents.

### 5.2 Hash input

Object hashes MUST be domain-separated:

```text
H(
  "AVC-OBJECT\0" ||
  format_version ||
  object_type ||
  canonical_payload_length ||
  canonical_payload
)
```

This prevents ambiguous cross-type identities and leaves room for object-format evolution.

**[OPEN] Prototype default:** BLAKE3-256 for local speed and parallelism, with SHA-256 supported by the ID envelope. Freeze the default only after a security/interoperability review and representative hashing benchmarks. The repository format MUST carry the algorithm code from the first implementation.

### 5.3 Canonical payload codec

Content-addressed objects MUST NOT use ordinary `serde` output as their identity format. The canonical codec SHOULD be deliberately small:

- fixed object field order;
- unsigned integers encoded with minimal unsigned LEB128;
- byte strings encoded as length plus bytes;
- arrays encoded as count plus canonical elements;
- maps encoded in canonical key-byte order;
- no floating-point values;
- no optional-field ambiguity;
- duplicate map keys rejected;
- non-minimal integers rejected;
- unknown required fields rejected;
- explicit version and type tags; and
- parser allocation, nesting, object-size, and element-count limits.

Canonical object encoding and external RPC encoding are separate. RPC MAY use Protobuf, JSON, or another versioned schema; it MUST NOT define object identity.

### 5.4 Text form

Human-facing IDs SHOULD use a checksummed lowercase base32 representation with explicit type prefixes, for example:

```text
chg_4q2m…
rev_b3_k7zp…
op_b3_n0fd…
```

Commands MAY accept unambiguous prefixes. Machine APIs MUST use complete IDs.

---

## 6. Canonical object model

The following is logical pseudocode, not a commitment to Rust memory layout.

### 6.1 File content

```rust
enum FileContent {
    Inline(Bytes),
    WholeBlob(BlobId),
    Chunked {
        total_length: u64,
        chunks: ChunkMapId,
    },
}

enum FileKind {
    Regular { executable: bool },
    Symlink,
}

struct FileManifest {
    format_version: u32,
    kind: FileKind,
    content: FileContent,
}
```

Version 1 intentionally stores only source-control-relevant metadata:

- regular file or symbolic link;
- executable bit where meaningful; and
- content.

Modification time, ownership, ACLs, platform-specific flags, sparse-allocation layout, and arbitrary extended attributes are excluded from v1. Hard links are captured as independent regular-file entries. Device nodes, sockets, and FIFOs are rejected by default rather than recreated on checkout. Additional portable metadata requires an ADR and a new manifest format.

### 6.2 Directory entries

```rust
enum ResolvedEntry {
    File(FileManifestId),
    Directory(DirectoryId),
}

enum DirectoryTarget {
    Resolved(ResolvedEntry),
    Conflict(ConflictId),
}

struct DirectoryEntry {
    name: PathAtom,
    target: DirectoryTarget,
}
```

Entries are ordered by canonical path-component bytes. A path component MUST NOT be empty, `.` or `..`, and MUST NOT contain a separator or NUL.

### 6.3 Directory map

AVC uses a hierarchical directory graph: a parent directory contains one entry referring to a child directory. It does not use one canonical global map keyed by full paths. This keeps subtree identity stable when a directory is moved or renamed.

```rust
enum Directory {
    Flat {
        entries: Vec<DirectoryEntry>,
    },
    Prolly {
        root: DirectoryNodeId,
        parameters: ProllyFormatId,
    },
}
```

**[DECISION] Small directories SHOULD use one sorted flat object. Large directories SHOULD use a deterministic, history-independent paged Merkle map.**

**[OPEN] Do not freeze the large-directory algorithm before the Phase 0 structure benchmark.** Compare at least:

1. flat Git-like trees;
2. immutable copy-on-write B+ trees;
3. deterministic POS/prolly-style trees; and
4. a canonical Merkle search-tree/treap baseline.

Selection criteria:

- identical logical maps produce identical roots regardless of update order;
- single-entry updates rewrite data proportional to tree height, not directory size;
- diff traverses equal subtrees by ID and scales with changed nodes;
- pages have useful physical locality;
- pathological keys cannot create unbounded depth or memory use;
- implementation and verification complexity remain acceptable.

ForkBase’s POS-Tree combines content-defined boundaries, Merkle hashing, and a B+-tree-like index to make multiversion pages reusable independent of update history ([ForkBase paper, 2018](https://www.vldb.org/pvldb/vol11/p1137-wang.pdf)). Dolt’s prolly-tree work is a production-oriented implementation reference, but its parameters MUST NOT be copied without AVC-specific measurements ([Dolt prolly chunker](https://www.dolthub.com/blog/2022-06-27-prolly-chunker/)).

### 6.4 Conflict representation

Conflicts are logical objects, not only marker text in a working copy.

Use a normalized multiway merge-term representation inspired by Jujutsu’s conflict model:

```rust
enum EntryTerm {
    Absent,
    File(FileManifestId),
    Directory(DirectoryId),
}

struct MergeTerms<T> {
    removes: Vec<T>,
    adds: Vec<T>,
}

struct Conflict {
    format_version: u32,
    terms: MergeTerms<EntryTerm>,
    provenance: Vec<RevisionId>,
}
```

For an ordinary three-way conflict:

```text
removes = [base]
adds    = [left, right]
```

Equal positive and negative terms cancel during normalization. Terms MUST be placed in canonical object-byte order after simplification; multiplicity is preserved where it remains semantically meaningful. Merging a revision that already contains conflicts MUST flatten and simplify terms rather than nest textual conflict markers. The implementation may initially support the common three-way case while preserving an extensible n-way encoding.

Jujutsu demonstrates that conflicted states can be recorded in commits and further rebased or merged rather than forcing an operation-specific `--continue` state ([Jujutsu conflict model](https://jj-vcs.github.io/jj/latest/conflicts/)).

### 6.5 Revision

```rust
struct Revision {
    format_version: u32,
    change_id: ChangeId,
    root: DirectoryId,
    parents: Vec<RevisionId>,
    description: String,
    author: Identity,
    merge_info: Option<MergeInfo>,
    metadata: SortedMap<String, Bytes>,
}

struct MergeInfo {
    base_revisions: Vec<RevisionId>,
    engine: MergeEngineIdentity,
    file_driver_set: Vec<MergeDriverIdentity>,
}
```

Rules:

- Parent IDs MUST be unique and canonically ordered where parent order has no user-visible meaning. If first-parent semantics are supported, that ordering MUST be explicit in the format.
- Wall-clock recording time SHOULD live in the operation, not the revision. This avoids changing content identity only because a snapshot occurred later.
- Merge/rebase results MUST record the deterministic merge engine and relevant driver identities in `merge_info`; the field is absent for ordinary non-merge revisions.
- Metadata keys MUST be namespaced and bounded.
- A normal edit of an existing change creates a new revision with the same `ChangeId` and semantic parents.
- Rebase creates a new revision with the same `ChangeId`, a new parent set, and usually a new root after merge.
- A merge normally creates a new `ChangeId` and a revision with multiple parents.

### 6.6 Repository view

A view is the complete logical repository state at an operation.

```rust
struct RepositoryView {
    format_version: u32,
    visible_heads: MergeTerms<RevisionId>,
    public_names: PersistentMap<Name, RefTarget>,
    workspaces: PersistentMap<WorkspaceId, WorkspaceView>,
    checkpoints: PersistentMap<CheckpointName, CheckpointTarget>,
    remote_views: PersistentMap<RemoteName, RemoteViewId>,
}

struct WorkspaceView {
    current_revision: RevisionId,
    materialization_profile: ProfileId,
    draft_remote: Option<DraftRef>,
}
```

`RefTarget` SHOULD be conflict-capable rather than assuming every concurrent name update can be reduced immediately to one revision. Most views remain conflict-free.

The view should not contain a flat copy of every historical revision. Revision visibility, `ChangeId` lookup, generations, and reachability are accelerated by derived indexes.

### 6.7 Operation

```rust
struct Operation {
    format_version: u32,
    epoch: OperationEpochId,
    parents: Vec<OperationId>,
    view: ViewId,
    actor: Identity,
    recorded_at: Timestamp,
    workspace: Option<WorkspaceId>,
    kind: OperationKind,
    summary: String,
    details: Bytes,
}
```

The operation graph is normally linear locally but can represent divergence. Every command that changes repository state commits one complete view.

### 6.8 Operation epochs

An unbounded operation chain would retain every automatic savepoint and all content reachable from old views. AVC therefore needs pruneable operation epochs.

```rust
struct OperationEpoch {
    format_version: u32,
    initial_view: ViewId,
    previous_epoch_digest: Option<Digest>,
    retention_started_at: Timestamp,
}
```

When retention compacts history:

1. create a new epoch whose initial view is the retained current view;
2. preserve named checkpoints and explicitly pinned revisions as independent roots;
3. record the digest of the prior epoch for audit continuity without making it a live object reference; and
4. allow old epoch objects to be deleted according to policy.

The MVP MAY defer actual deletion, but the format MUST NOT make indefinite retention unavoidable.

### 6.9 Draft and publication manifests

```rust
struct DraftManifest {
    repository: RepositoryId,
    workspace: WorkspaceId,
    expected_previous: Option<DraftManifestId>,
    view: ViewId,
    expires_at: Option<Timestamp>,
    idempotency_key: IdempotencyKey,
}

struct ProposalManifest {
    repository: RepositoryId,
    proposal: ProposalId,
    expected_previous: Option<ProposalManifestId>,
    base_public_view: PublicViewId,
    proposed_view: ViewId,
    proposed_revisions: Vec<RevisionId>,
    policy: ValidationPolicyId,
    actor: Identity,
    idempotency_key: IdempotencyKey,
}

struct PublicationManifest {
    repository: RepositoryId,
    expected_public_view: PublicViewId,
    new_public_view: PublicViewId,
    published_revisions: Vec<RevisionId>,
    policy: ValidationPolicyId,
    actor: Identity,
    idempotency_key: IdempotencyKey,
    signature: Option<Signature>,
}
```

The manifests are distinct from the uploaded object bundle. A proposal makes selected draft revisions reviewable without advancing a public channel; it is collaboration metadata, not a new source revision. A failed compare-and-swap proposal or publication MUST NOT require re-uploading already accepted immutable objects. If signatures are enabled, the signature covers a domain-separated canonical manifest payload with the signature field omitted.

---

## 7. Path and filesystem model

### 7.1 Portable path profile

**[DECISION] New repositories SHOULD default to a portable path profile.**

The profile SHOULD:

- require valid UTF-8 path components;
- use a repository-versioned Unicode normalization/case-folding specification;
- reject names that collide under the portable comparison key;
- reject Windows reserved device names and invalid suffixes;
- reject NUL, separators, `.` and `..`; and
- preserve the original canonical display spelling.

An explicit Unix-bytes profile MAY be added later for repositories that accept reduced portability. The exact Unicode version and collision algorithm require an ADR and cross-platform test corpus before format freeze.

### 7.2 Safe checkout

Checkout MUST resist path traversal and symlink attacks:

- never concatenate untrusted repository paths and blindly open them;
- use directory-relative, no-follow APIs where available;
- never follow a repository-created symlink while writing a descendant path;
- create temporary files in the target directory and atomically replace where supported;
- validate case/normalization collisions before materialization; and
- make partial checkout recovery idempotent.

### 7.3 Files changing during snapshot

For every candidate path:

1. read metadata;
2. open without following an unexpected symlink;
3. read and hash the content;
4. read metadata again;
5. retry if identity, size, or change indicators differ; and
6. report a busy-file condition after a bounded retry policy.

A normal userspace snapshot of multiple files is a coherent observation, not a guaranteed application transaction. AVC MUST document this distinction.

- **Automatic savepoint:** a debounced coherent observation.
- **Explicit checkpoint:** a user/tool-requested barrier after coordinated writes complete.
- **Filesystem-assisted checkpoint:** stronger semantics when an underlying snapshot API is available.

---

## 8. Append-only storage engine

### 8.1 Design objective

A naive implementation can write content into a journal, then an object database, then a database WAL, then a compacted pack. AVC SHOULD avoid that log-on-log pattern.

**[DECISION] New object payloads and transaction records MUST enter through one append-only segment path.** XLL’s cross-layer logging work demonstrates the potential cost of duplicated durability layers, although its TiKV results are not directly transferable to source control ([XLL, NSDI 2026](https://www.usenix.org/conference/nsdi26/presentation/shawger)).

### 8.2 Segment record

Conceptual framing:

```rust
struct SegmentRecordHeader {
    magic: [u8; 4],
    record_version: u16,
    record_type: RecordType,
    object_type: Option<ObjectType>,
    hash_algorithm: Option<HashAlgorithm>,
    compression: Compression,
    canonical_length: u64,
    stored_length: u64,
    object_id: Option<ObjectId>,
    crc32c: u32,
}
```

The object hash covers the canonical uncompressed payload. CRC32C protects the stored record framing and compressed bytes against accidental corruption. On read, AVC verifies record framing, decompression limits, canonical length, and content hash before returning trusted content.

### 8.3 Transaction protocol

A repository mutation writes:

```text
BEGIN_TX(tx_id, previous_operation_heads)
OBJECT(...)
OBJECT(...)
VIEW(...)
OPERATION(...)
COMMIT_TX(tx_id, resulting_operation_id, transaction_digest)
fdatasync(active_segment)          # or the platform durable equivalent
atomic_replace(state/heads)
fsync(state_directory)             # or the platform durable equivalent
```

Required ordering:

1. all referenced object records are appended;
2. the terminal commit record is appended;
3. the segment is durably flushed;
4. only then may the current operation-head file reference the operation.

Crash outcomes:

- Before a valid `COMMIT_TX`: appended tail records are unreachable and ignored.
- After segment durability but before head update: the committed transaction is durable but not current. If recovery finds exactly one valid committed successor chain from the recorded heads, it SHOULD advance to it automatically. If it finds divergent successors, it preserves them as an operation-head set and requires deterministic view reconciliation.
- After atomic head update: the operation and its complete closure are visible.

Recovery MUST tolerate truncation at every byte boundary. It scans the active segment until the last complete valid record/transaction and truncates or ignores the remaining tail. The small `state/heads` file is an acceleration/root pointer, not the only copy of repository state; it can be reconstructed from valid committed transactions.

### 8.4 Sealing segments

When the active segment reaches its configured target:

1. stop appending under the write lock;
2. write a footer containing record counts, offsets, checksums, and an object-ID index;
3. flush the file;
4. atomically rename it into `segments/`;
5. create and flush the next active segment; and
6. atomically update the store manifest.

The footer MAY include:

- sorted object ID to offset table;
- compact fan-out table;
- Bloom filter;
- per-object-type ranges; and
- segment-level checksum.

### 8.5 Object-location index

The active segment uses an in-memory index reconstructed at startup. Sealed segment indexes are immutable. A global multi-segment index MAY map object IDs to the newest/preferred physical copy, analogous in role—not format—to Git’s multi-pack index ([Git multi-pack-index/commit-graph configuration](https://git-scm.com/docs/git-config)).

The global index is a cache. If missing or corrupt, scan sealed footers and rebuild it.

### 8.6 Compression and physical deltas

MVP:

- compress object payloads independently with a bounded, well-supported codec;
- do not create cross-object delta chains;
- cap decompressed size and expansion ratio; and
- retain direct random access.

Later compaction MAY delta-compress related whole-file blobs. Delta depth MUST be bounded, base selection MUST be deterministic or purely physical, and reconstruction MUST verify the final logical object hash. Physical delta choices never affect `BlobId`.

### 8.7 Compaction

Compaction:

1. captures a stable root set and reader leases;
2. marks reachable objects;
3. writes selected reachable objects into new immutable segments;
4. builds new indexes;
5. flushes and verifies them;
6. atomically replaces the segment manifest; and
7. deletes retired segments only after old reader leases expire.

An interrupted compaction MUST leave either the old manifest or the new manifest usable. It MUST NOT require in-place mutation of sealed segments.

### 8.8 Garbage-collection roots

Roots include:

- current operation heads and retained operations;
- current workspace views;
- named checkpoints;
- public views;
- non-expired draft manifests;
- pinned revisions;
- remote-tracking roots required by policy;
- active upload/import transactions; and
- reader/checkout leases.

Automatic savepoint retention and operation-epoch pruning determine when obsolete revision versions become unreachable.

---

## 9. File-content strategy

### 9.1 MVP behavior

The MVP MUST support inline and whole-blob content. It SHOULD define the chunked form in the logical schema but MUST NOT emit chunked files until the threshold and algorithm are benchmarked.

Whole blobs are preferable for ordinary source files because they provide:

- fewer objects and lookups;
- simple hashing and verification;
- straightforward merge inputs;
- simpler corruption recovery; and
- compatibility with later segment-level physical delta compression.

### 9.2 Selective content-defined chunking

CDC is appropriate only when large-file versions retain substantial internal similarity despite insertions or shifts. Candidate workloads include large generated data, disk/database images, large structured assets, and selected binary formats.

VectorCDC reports that full-file CDC scanning can itself be a bottleneck and demonstrates substantial vectorized acceleration in its evaluated deduplication workloads ([FAST 2025](https://www.usenix.org/conference/fast25/presentation/udayashankar)). AVC therefore MUST compare CDC against:

- whole blobs plus compression;
- whole blobs plus physical delta compression;
- fixed-size chunks; and
- content-defined chunks.

The comparison must include read amplification, object count, indexing cost, update cost, transfer bytes, corruption recovery, and adversarial inputs—not storage ratio alone.

### 9.3 CDC security

Cross-user deduplication can reveal content equality. Keyed CDC also has nontrivial cryptographic pitfalls; a 2025 analysis describes key-recovery and fingerprinting weaknesses in several schemes ([Breaking and Fixing Content-Defined Chunking](https://eprint.iacr.org/2025/558)).

Therefore:

- AVC MUST NOT enable global cross-tenant deduplication by default.
- Deduplication SHOULD be scoped to a repository or trusted tenant.
- Client-side encrypted storage requires a separate threat model and protocol.
- Chunking parameters and seeds MUST be format-versioned.

---

## 10. Working-copy engine and automatic savepoints

### 10.1 Working-copy state

Each workspace keeps a rebuildable dirstate containing, per tracked path:

- path identity;
- file kind;
- filesystem file ID/inode where available;
- size;
- timestamp/change token;
- last verified content ID;
- conflict materialization state; and
- sparse/materialization state.

Jujutsu’s `TreeState` similarly tracks working-copy tree identity plus mtime and size to accelerate snapshots ([Jujutsu architecture](https://jj-vcs.github.io/jj/latest/technical/architecture/)). Sapling uses filesystem monitoring and incremental working-copy state to avoid operations proportional to all files ([Sapling scale overview](https://sapling-scm.com/docs/scale/overview/)).

### 10.2 Change-information sources

The working-copy engine combines:

1. filesystem watcher events;
2. editor/IDE edit notifications;
3. explicit tool checkpoints;
4. cached dirstate metadata;
5. optional filesystem journals or write-range APIs; and
6. periodic reconciliation scans.

SolFS shows how write-offset logs can identify changed file regions without whole-file hash discovery, and SkySync shows how existing storage checksums can reduce duplicate checksum work. AVC MAY exploit equivalent information where available, but it MUST verify resulting content IDs before committing snapshots ([SolFS](https://www.usenix.org/conference/atc25/presentation/pan), [SkySync](https://www.usenix.org/conference/fast26/presentation/zhang-zhihao)).

### 10.3 Watcher correctness

Watcher events are advisory. Queues can overflow; Watchman recovers from lost synchronization by recrawling the tree ([Watchman troubleshooting](https://facebook.github.io/watchman/docs/troubleshooting)).

AVC MUST maintain a watcher confidence state:

```text
Clean       all events since last verification are believed observed
Dirty       candidate paths need verification
Unknown     overflow, root replacement, clock anomaly, or state corruption; recrawl required
```

No-change `status` may be O(1) only while the watcher/dirstate state is `Clean`. `Unknown` requires a full or profile-bounded reconciliation.

### 10.4 Snapshot algorithm

```text
snapshot(workspace):
    acquire repository write transaction
    load workspace base revision and dirstate
    gather candidate paths from watcher + explicit dirtiness

    if confidence == Unknown:
        reconcile the materialized path set

    for each candidate path in canonical order:
        safely inspect the filesystem entry
        if unchanged by trusted metadata, reuse prior entry ID
        otherwise read/hash with pre/post metadata validation
        update only directory nodes on the changed path

    if new_root == current_revision.root and revision metadata unchanged:
        update dirstate only; do not create a revision or operation
    else:
        create replacement Revision with same ChangeId and parents
        update workspace View
        append objects, View, Operation, and transaction commit
        update dirstate after repository commit
```

If the repository transaction commits but dirstate update fails, the next command reconciles from the committed revision. Dirstate is never authoritative.

### 10.5 Debounce policy

Automatic savepoints SHOULD use both:

- a quiet interval after recent writes; and
- a maximum interval so continuous editing still receives recovery points.

Initial values are configuration defaults, not format decisions. The daemon SHOULD coalesce editor temporary-file/rename sequences and SHOULD skip no-op tree roots.

Useful trigger sources:

- edit quiescence;
- before a checkout/rebase/merge/split/fold;
- successful build/test notification;
- explicit `avc checkpoint`;
- workspace deactivation; and
- daemon shutdown.

### 10.6 Retention

Retention MUST distinguish:

- published revisions: retained by public policy;
- named checkpoints: retained until removed;
- current visible change revisions: retained;
- automatic savepoints: compactable; and
- abandoned drafts: expirable.

A default tiered savepoint policy MAY retain dense recent history and progressively coarser older history. Exact windows MUST be configurable and should be selected after measuring real edit sessions. `avc gc --explain` MUST report why an object remains reachable.

---

## 11. Change and history semantics

### 11.1 Beginning and finishing work

A workspace always edits one current change revision.

```text
avc new "Add streaming parser"
# edit; automatic savepoints replace this revision
avc finish
```

`finish` marks the current change ready and creates a new empty change on top. A familiar convenience command MAY exist:

```text
avc commit -m "Add streaming parser"
```

In AVC, `commit` means “describe and finish this logical change,” not “make my files durable.” Durability already comes from savepoints.

### 11.2 Rewriting

Operations include:

- `split`: divide one change into multiple changes;
- `fold`: combine changes;
- `move`: move selected paths or hunks between changes;
- `reorder`: change stack order;
- `rebase`: replay logical changes on new parents while retaining change IDs;
- `abandon`: remove a change from the visible view; and
- `restore`: recover content or a complete earlier repository view.

Every operation produces replacement revisions as needed and one new operation/view. Old revisions remain recoverable while retained by the operation policy.

### 11.3 Descendant rewriting

When a revision is replaced, visible descendants based on it SHOULD be automatically rebased in topological order, as Jujutsu does. The operation is atomic from the repository-view perspective:

- all replacements and conflicts become visible together; or
- none become visible.

A conflict does not abort the graph rewrite. It produces conflict-bearing replacement revisions.

### 11.4 Undo and restore

```text
avc undo                 # invert/restore the latest user operation
avc redo                 # reapply an immediately undone operation when unambiguous
avc op log               # inspect repository operations
avc op show <id>
avc op restore <id>      # append a new operation using that operation's view
avc restore <rev> -- <paths>
```

Undo MUST append a new operation. It MUST NOT mutate or delete the operation being undone. Published history is not silently rewound; reverting a publication creates a new publication selecting prior or inverse content.

---

## 12. Merge and conflict design

### 12.1 Baseline merge

The trusted core MUST provide deterministic:

- revision merge-base selection;
- recursive/virtual-base handling for multiple best common ancestors;
- tree merge using object-ID equality and recursive directory traversal;
- regular text three-way merge;
- binary/type/mode conflict production; and
- conflict-object normalization.

Tree traversal skips equal subtrees by ID. If only one side differs from the base, take the changed side. If both sides resolve to the same ID, take it. Recurse only when necessary.

### 12.2 Rename detection

MVP order:

1. exact content-ID move/rename matching;
2. explicit move intent recorded by AVC operations, when trustworthy;
3. otherwise represent delete/add or conflict.

Similarity-based rename detection MAY be added later as a bounded heuristic. It MUST NOT affect revision identity nondeterministically across machines.

### 12.3 Text merge

The baseline merge driver MUST be deterministic and versioned. Its identity/version MUST be recorded when a merge result is materialized so behavior can be reproduced.

Do not claim semantic correctness. Successful textual merge means the algorithm found no textual conflict; validation belongs to parsers, tests, and review.

### 12.4 Structured merge drivers

Mergiraf provides a useful architecture: run a fast line merge first, invoke syntax-aware merging only when useful, and retain fallback behavior ([Mergiraf architecture](https://mergiraf.org/architecture.html)). AVC SHOULD support versioned merge-driver plugins with:

- declared path/language applicability;
- deterministic inputs and outputs;
- resource limits;
- driver identity and version;
- no network access by default;
- a safe textual fallback; and
- a result classified as resolved, conflicted, or failed.

AST/Tree-sitter and model-assisted resolution are optional proposal mechanisms. They MUST NOT silently define canonical source truth without deterministic validation.

### 12.5 Working-copy conflict materialization

The repository stores logical conflict objects. The workspace may render marker files for tools and editors.

MVP sidecar approach:

- store `path -> ConflictId` and the hash of materialized marker content in workspace state;
- if a scan sees the same marker-content hash, preserve the conflict object;
- if the file changed, treat the file as a user-provided whole-file resolution unless `avc resolve` provides finer structured information; and
- never reconstruct conflict provenance solely by parsing arbitrary marker text.

A later marker format MAY support partial resolution, but the sidecar remains authoritative for unresolved terms.

### 12.6 Publication policy

The AVC object model permits conflicted revisions. Default public-channel policy SHOULD reject unresolved conflicts. Draft sync and review proposals MAY allow them.

---

## 13. Workspaces and materialization

### 13.1 First-class workspaces

```text
avc workspace new experiment
avc workspace switch experiment
avc workspace list
avc workspace remove experiment
```

A new workspace creates metadata immediately and materializes files according to its profile. Multiple workspaces share immutable objects.

### 13.2 Materialization profiles

Profiles MAY specify:

- all paths;
- explicit path prefixes;
- project/module sets;
- generated-file exclusions; and
- on-demand paths.

The object API MUST be lazy-ready from the start:

```text
get_directory(id)
get_file_manifest(id)
get_blob(id, optional_range)
materialize(path)
prefetch(paths)
get_content_id(path)
```

### 13.3 Normal filesystem first

EdenFS shows that lazy virtual working copies can make massive monorepos usable, but also that implementation is OS-specific: its public overview describes FUSE on Linux, NFS on macOS, and projected-filesystem mechanisms on Windows ([EdenFS overview](https://github.com/facebook/sapling/blob/main/eden/fs/docs/Overview.md)). AVC therefore starts with an ordinary directory.

It MAY accelerate workspace creation using platform copy-on-write clone/reflink APIs, but correctness cannot depend on them.

### 13.4 Future virtual workspace

A later optional VFS may provide:

- lazy file fetch;
- fast checkout transitions;
- content IDs without re-reading materialized files;
- filesystem notifications; and
- cheap speculative workspaces.

BranchFS is a relevant 2026 preprint/prototype for copy-on-write branches with atomic commit/abort, but it is early research and agent-oriented; it is a future integration reference, not AVC’s storage foundation ([paper](https://arxiv.org/abs/2602.08199), [source](https://github.com/multikernel/branchfs)).

---

## 14. Synchronization and server design

### 14.1 Two remote operations

#### Draft synchronization

Draft sync uploads missing immutable objects and advances a private or restricted workspace root.

By default it MUST NOT:

- create a public branch/name;
- open or update a review;
- send project webhooks;
- emit broad activity-feed events;
- trigger full project CI; or
- reindex the public code-search corpus.

It can support explicit lightweight validation and retention/expiration policies.

#### Publication

Publication atomically advances shared project state and is the normal event boundary for review, CI, indexing, notifications, and integrations.

### 14.2 Protocol sequence

Initial centralized protocol:

```text
1. capability exchange
2. authenticate repository and actor
3. exchange current public/draft root IDs
4. compute a closure relative to roots the server confirms it has
5. stream one or more framed object bundles into a quarantine segment
6. validate framing, hashes, quotas, object graph, and policy
7. submit DraftManifest, ProposalManifest, or PublicationManifest
8. compare-and-swap the expected prior manifest/view
9. atomically attach the already-verified segment and new root
10. emit one typed transition event
```

Objects SHOULD be sent child-before-parent where practical, but the server must validate complete closure before accepting a root.

### 14.3 Bundle format

The bundle SHOULD reuse the segment record envelope so the server can retain verified uploaded bytes without rewriting them into another log. A bundle contains:

- format/capability header;
- object records;
- optional indexes;
- claimed root set;
- transaction digest; and
- optional sender signature.

The server writes directly to a transaction-scoped quarantine segment. Acceptance attaches the segment to the tenant store by manifest update. Rejection or expiration deletes it. This applies the cross-layer logging principle without weakening quarantine.

### 14.4 Missing-object discovery

MVP:

- client starts from selected roots;
- server confirms known base roots;
- client excludes objects reachable from those bases;
- duplicate uploads are harmless because IDs are content-addressed; and
- server reports any missing closure after validation.

Later, set reconciliation MAY help when stores overlap heavily but lack a convenient shared root. Rateless IBLTs are a relevant SIGCOMM 2024 technique for unknown set-difference sizes ([ACM paper](https://dl.acm.org/doi/10.1145/3651890.3672219)). They do not solve object transfer, authorization, reachability, or publication conflicts and are not an MVP requirement.

### 14.5 Atomic public view

Server state contains a content-addressed `PublicView` and a small authoritative pointer to its current ID.

Publish succeeds only when:

```text
manifest.expected_public_view == server.current_public_view
```

Otherwise the server returns:

- current view ID;
- conflicting public-name updates;
- potentially reusable uploaded object transaction ID; and
- a structured recommendation to fetch/rebase/merge.

Repeated requests with the same idempotency key return the original result.

### 14.6 Server storage

A minimal server requires:

- tenant-scoped immutable object segments;
- draft/public manifest database;
- authorization and quota service;
- object/graph validation workers;
- compaction and GC;
- event outbox; and
- read APIs for fetch and browsing.

It does not require a full forge. Review, search, CI, and notification services consume typed publication/proposal events. The public/proposal metadata transition and its event-outbox record MUST commit atomically after the referenced object segment is durable. Delivery is at least once; every event has a stable ID and consumers MUST be idempotent.

### 14.7 CI semantics

Recommended policy levels:

```text
savepoint    no remote CI
sync         optional explicit targeted checks
proposal     review validation policy
publish      canonical required checks
release      deployment/release checks
```

A host cannot reduce CI amplification if every private edit is still translated into a public push. AVC’s client, server, and event model must all preserve the savepoint/draft/publication distinction.

---

## 15. Build outputs, generated files, and artifacts

Source history MUST not become the default store for build logs, test output, generated binaries, or large transient artifacts.

Bazel’s remote cache separates an action cache from a content-addressable store of outputs ([Bazel remote caching](https://bazel.build/remote/caching)). AVC SHOULD expose stable source-root IDs that build systems can use in action keys while keeping artifacts in a separate service.

Conceptually:

```text
ActionKey = H(
    selected source tree/revision,
    declared build inputs,
    toolchain identity,
    command,
    environment manifest
)
```

Artifact storage and source storage SHOULD have separate:

- retention;
- quotas;
- access control;
- replication;
- encryption policy; and
- garbage collection.

A revision or publication MAY reference an artifact digest or attestation, but the artifact payload is not part of the source-tree object graph unless deliberately versioned as source.

---

## 16. CLI and machine API

### 16.1 CLI principles

- No required staging index.
- New files are tracked according to explicit repository auto-track/ignore policy.
- Commands operate on changes and workspaces, not hidden intermediate state.
- Every mutating command is one operation and can be explained before execution.
- Default output shows relevant local changes plus nearby shared context.
- Stable IDs are accepted everywhere.
- Interactive selection is optional; noninteractive equivalents always exist.

### 16.2 Proposed command surface

```text
# Repository and inspection
avc init
avc clone <remote>
avc status
avc diff [revision-or-change]
avc log
avc show <id>

# Logical changes
avc new [description]
avc describe [change] <text>
avc finish
avc commit -m <text>              # convenience: describe + finish
avc split [change]
avc fold <source> --into <target>
avc move <paths-or-selection> --to <change>
avc reorder <change> --before|--after <other>
avc abandon <change>
avc rebase <changes> --onto <revision>
avc merge <revisions>

# Recovery
avc checkpoint [name]
avc undo
avc redo
avc restore <revision-or-checkpoint> [-- <paths>]
avc op log
avc op show <operation>
avc op restore <operation>

# Workspaces
avc workspace new <name>
avc workspace switch <name>
avc workspace list
avc workspace remove <name>

# Collaboration
avc sync [remote]
avc propose [changes]
avc publish [changes] [--channel <name>]
avc fetch [remote]

# Maintenance
avc fsck
avc gc [--explain]
avc debug store
avc debug rebuild-indexes
```

Names can change after usability testing; the concepts should not.

### 16.3 Status example

```text
Workspace: default
Current change: chg_q2ma  Add streaming parser
Base: main@rev_7c81

Modified:
  src/parser.rs
  src/stream.rs

Untracked by policy:
  notes/benchmark.txt

Recovery: savepoint 18s ago
Draft sync: not configured
```

### 16.4 Log example

```text
@  chg_q2ma  Add streaming parser                 [editing]
│
○  chg_r91f  Introduce byte-source abstraction    [ready]
│
◆  main      rev_7c81  Published 2026-08-12
```

### 16.5 Machine API

The CLI SHOULD be a client of the same typed repository service used by IDEs and automation.

The API MUST provide:

- version negotiation;
- stable request/response schemas;
- full IDs;
- structured error codes;
- idempotency keys for mutating remote requests;
- expected-operation and expected-public-view preconditions;
- noninteractive operation;
- atomic compound operations;
- dry-run/explain output;
- deterministic pagination;
- capability discovery; and
- transaction-scoped diagnostics.

Example:

```json
{
  "api_version": 1,
  "operation": "publish",
  "repository": "repo_...",
  "changes": ["chg_q2ma..."],
  "expected_public_view": "view_b3_...",
  "channel": "main",
  "validation_policy": "review-default",
  "idempotency_key": "01J..."
}
```

Shell-output parsing is not an API.

---

## 17. Security and trust model

### 17.1 Untrusted inputs

Treat as untrusted:

- repository object bytes;
- bundles;
- compression streams;
- object graphs;
- path names;
- merge-driver output;
- remote metadata;
- signatures/certificates;
- working-copy files modified by other processes; and
- derived indexes after crashes.

Parsers MUST have explicit limits for object size, nesting, entry count, path length, decompression ratio, graph traversal, and CPU work.

### 17.2 Object integrity versus authenticity

Content hashes prove that bytes match an ID; they do not prove who created or authorized the object. Authorization belongs to authenticated server operations and publication signatures/audit records.

### 17.3 Draft privacy and secret retention

Automatic history can retain:

- deleted credentials;
- temporary secrets;
- private prompts/context;
- personal data;
- ignored build output; and
- files users believed were never recorded.

Therefore:

- ignore/track policy MUST be applied before durable object insertion;
- secret/path exclusion rules SHOULD be available locally;
- `avc status --why-tracked <path>` SHOULD explain policy;
- draft sync MUST be opt-in/configured and access-controlled;
- retention controls MUST be visible;
- deletion documentation MUST distinguish local GC from replicas/backups; and
- a later secret-redaction/admin-erasure tool must be explicit and auditable.

### 17.4 Deduplication boundary

Do not deduplicate across mutually untrusted tenants by default. Repository/tenant-scoped dedup avoids many equality side channels and simplifies deletion/accounting.

### 17.5 Merge plugins

Run merge plugins with:

- no network by default;
- bounded CPU/memory/output;
- read-only access to declared inputs;
- deterministic locale/environment;
- explicit version identity; and
- validation of returned canonical data.

---

## 18. Correctness invariants

These invariants are release blockers.

1. **Complete visible closure.** Every object reachable from a visible revision/view is present and hash-valid.
2. **Atomic transaction visibility.** After a crash, a transaction is absent or completely visible, never partially visible.
3. **Disposable indexes.** Removing derived indexes cannot lose source or repository state.
4. **Idempotent recovery.** Re-running recovery produces the same state.
5. **Safe GC.** No object reachable from a retained root or active lease is deleted.
6. **Atomic publication.** At most one incompatible manifest succeeds for a given expected public view.
7. **Idempotent upload/publication.** Duplicate objects and repeated idempotency keys do not create distinct semantic results.
8. **Verified reads.** Corrupt bytes are detected before being returned as valid objects.
9. **Safe compaction.** Interruption leaves an old or new complete segment manifest.
10. **Undo recovery.** Every retained operation view can be restored without relying on mutable cache state.
11. **Canonical identity.** Equivalent logical objects encode identically; noncanonical encodings are rejected.
12. **Deterministic merge.** Same inputs, driver version, and configuration produce the same merge object.
13. **Path safety.** Repository paths cannot escape or redirect checkout writes outside the workspace.
14. **No watcher-only truth.** Missing filesystem events cannot permanently hide a working-copy change.

---

## 19. Verification and testing strategy

### 19.1 Reference model

Build a deliberately slow in-memory reference repository before optimizing storage. It should model:

- canonical trees;
- revisions and changes;
- views and operations;
- rewrite and undo;
- merge/conflict terms;
- retention roots; and
- publication compare-and-swap.

Optimized implementations are differentially tested against it.

Metis demonstrates the value of exploring filesystem inputs/states against a reference model rather than relying only on example tests ([FAST 2024](https://www.usenix.org/conference/fast24/presentation/liu-yifei)).

### 19.2 Property tests

Properties include:

- encode/decode round-trip;
- decode rejects noncanonical forms;
- object ID stable across process/platform;
- directory map independent of insertion order;
- applying no changes preserves root ID;
- tree diff followed by apply reconstructs target;
- merge symmetry where semantics allow;
- conflict-term normalization idempotent;
- undo restores prior view;
- operation restore reproduces target view;
- compaction preserves all reachable object bytes and IDs;
- GC never removes marked objects; and
- repeated remote request with one idempotency key has one result.

### 19.3 Crash injection

Use a deterministic storage shim that can fail after every:

- append;
- short write;
- flush;
- rename;
- directory sync;
- manifest update;
- segment seal;
- compaction phase; and
- head update.

For each failure point:

1. terminate without cleanup;
2. reopen and recover;
3. run `fsck`;
4. compare with the reference model’s allowed pre/post state; and
5. rerun recovery to prove idempotence.

### 19.4 Fuzzing

Fuzz:

- canonical object decoder;
- segment scanner;
- compression framing;
- bundle parser;
- tree diff/merge;
- conflict normalization;
- path validation;
- graph traversal limits; and
- RPC request validation.

No panic, unbounded allocation, path escape, or infinite traversal is acceptable for malformed input.

### 19.5 Formal models

Write small TLA+ or equivalent state-machine models for:

1. local transaction commit/head update;
2. segment-manifest compaction and reader leases;
3. draft/publication compare-and-swap;
4. operation divergence/merge; and
5. GC roots versus concurrent publication/upload.

Formal modeling is not a replacement for tests; it is focused on state transitions where rare interleavings can lose data.

### 19.6 Filesystem matrix

Test at minimum:

- Linux ext4 and a reflink-capable filesystem;
- macOS APFS, including case-insensitive default behavior;
- Windows NTFS;
- network/shared filesystem behavior where officially supported; and
- forced watcher overflow/reconciliation.

Document unsupported semantics rather than silently weakening correctness.

---

## 20. Performance model and proof obligations

AVC does not optimize only for “commits per second.” It measures total local, storage, synchronization, and platform cost.

### 20.1 Required asymptotic goals

| Operation | Intended scaling behavior |
|---|---|
| Clean warm `status` | O(1) with a trusted clean watcher token; otherwise O(candidate paths), with explicit recrawl fallback |
| Single-file savepoint | O(file bytes read + directory-map path updates), not O(repository files) |
| Tree equality | O(1) by root ID |
| Tree diff | O(changed Merkle nodes plus reported entries) |
| Log page | O(page size plus indexed graph queries), not O(total history) |
| Workspace metadata creation | O(1); materialization proportional to selected files |
| Sync | proportional to missing objects/content plus protocol metadata |
| Publish | one atomic public-view transaction per logical update |
| Undo | proportional to checkout/view transition, not reconstruction of command history |
| Index rebuild | linear in relevant immutable records; safely interruptible |

### 20.2 Focused benchmark suites

#### A. Directory structures

Datasets:

- realistic source trees;
- directories with 10, 1,000, 100,000, and 1,000,000 entries;
- repeated single-entry edits;
- bulk generated entries;
- independent insertion orders reaching the same map;
- directory rename/move; and
- sparse differences between large trees.

Measure object count, bytes written, update latency, diff latency, lookup latency, memory, page reuse, and cold/warm cache behavior.

#### B. Savepoints

Workloads:

- 1,000,000 revisions of one small file;
- many files changed one at a time;
- editor temporary-write/rename patterns;
- no-op events;
- continuous writers;
- large-file sparse edits; and
- repeated crashes.

Measure foreground latency, bytes read, bytes written, write amplification, storage before/after retention, recovery time, and status latency.

#### C. Working copies

Measure clean and dirty status, checkout/materialization, workspace creation, watcher overflow recovery, case-collision handling, and cold/warm filesystem cache across increasing file counts.

#### D. Merge/history

Use real and synthetic merge-heavy histories. Measure merge-base, tree merge, text merge, conflict count, graph rewrite latency, descendant rebase, and operation restore.

#### E. Synchronization

Compare:

- one sync per savepoint;
- batched draft sync;
- one publication per completed logical change;
- shared-base and divergent stores;
- duplicate bundle retry; and
- concurrent publication CAS conflicts.

Measure network bytes, server CPU/memory, segment bytes, transactions, event count, and retained draft storage.

#### F. Platform amplification

Per logical task count:

- draft syncs;
- public transitions;
- review updates;
- webhooks;
- notifications;
- CI jobs;
- CI compute time; and
- logs/artifacts retained.

This is the proof that publication semantics reduce downstream load; raw object-write speed cannot establish it.

### 20.3 Baselines

Compare the applicable subset against:

- Git with current maintenance features enabled;
- Jujutsu using its own UX over the relevant backend;
- Sapling where a reproducible public setup exists;
- ordinary filesystem copies/reflinks for workspace operations; and
- a simple flat-tree/whole-blob AVC reference implementation.

Set numerical gates only after recording baselines on declared hardware and filesystems. Publish dataset generators, exact commands, configuration, cache state, and raw results.

---

## 21. Rust implementation architecture

### 21.1 Initial crate

Keep the first working milestone in the existing Rust crate. Use modules with visible responsibility boundaries rather than creating a workspace before those boundaries are needed:

```text
src/
  id.rs          content and logical identifiers
  object.rs      blobs, flat trees, snapshots
  store.rs       replaceable physical object/state storage
  repo.rs        checkpoint, status, log, restore
  commands/      thin CLI adapters
```

Split crates only when a subsystem has a distinct consumer, lifecycle, or failure policy. Likely later boundaries are core objects, storage, repository semantics, working copy, merge, synchronization, and test support.

### 21.2 Dependency boundaries

- Logical object identity MUST NOT depend on physical object paths, compression, or state-file layout.
- Repository semantics SHOULD depend on a narrow object-store boundary, not scatter filesystem paths across commands.
- CLI modules SHOULD translate arguments/output and leave state transitions to the repository module.
- Async code SHOULD be restricted to a later daemon/server/transport boundary.
- Canonical encoding MUST NOT depend on RPC serialization libraries.
- Untrusted parsing MUST return typed errors, never panic.

### 21.3 Core traits

Illustrative boundaries:

```rust
trait ObjectStore {
    fn get(&self, id: &ObjectId) -> Result<CanonicalObject, StoreError>;
    fn contains(&self, id: &ObjectId) -> Result<bool, StoreError>;
    fn begin_write(&self) -> Result<Box<dyn WriteTransaction>, StoreError>;
}

trait WriteTransaction {
    fn put(&mut self, object: &CanonicalObject) -> Result<ObjectId, StoreError>;
    fn commit_operation(
        self: Box<Self>,
        operation: &Operation,
        expected_heads: &[OperationId],
    ) -> Result<CommitResult, StoreError>;
}

trait WorkingCopy {
    fn snapshot(&mut self, base: &Revision) -> Result<SnapshotResult, WorkingCopyError>;
    fn checkout(&mut self, target: &DirectoryId) -> Result<CheckoutResult, WorkingCopyError>;
}

trait MergeDriver {
    fn merge(&self, request: MergeRequest) -> Result<MergeResult, MergeError>;
    fn identity(&self) -> MergeDriverIdentity;
}
```

Exact APIs will evolve; maintain the separation between immutable logical data, physical storage, workspace observation, and transport.

### 21.4 Error model

Every command error should include:

- stable machine code;
- concise human message;
- structured context;
- whether retry is safe;
- any conflicting expected/actual IDs; and
- recovery/action suggestions generated from typed state, not string matching.

### 21.5 Unsafe code

Avoid unsafe code initially. If later required for memory mapping, SIMD, or platform filesystem APIs, isolate it behind a small audited module with fuzz/property tests and a documented safety contract.

---

## 22. Implementation roadmap

Build these as successive working milestones. Adjust their internal design as the implementation teaches us, while retaining the product model and correctness requirements.

### Phase 0 — Decisions and reference prototypes

Deliver:

- repository skeleton and CI;
- glossary and normative invariants;
- ADR template;
- in-memory reference model;
- canonical codec spike;
- directory-structure benchmark harness;
- append-segment crash model; and
- cross-platform path test corpus.

ADRs to decide before format freeze:

1. hash algorithm envelope and prototype default;
2. canonical codec;
3. path profile;
4. directory-map representation;
5. transaction/head durability protocol; and
6. conflict-term normalization.

**Exit criteria:** reference model can create trees/revisions/views/operations, rewrite one change, and restore an earlier operation; deterministic tests pass across supported platforms.

### Phase 1 — Durable explicit local snapshots

Implement:

- object IDs and canonical encoding;
- blob, file-manifest, flat-directory, revision, view, and operation objects;
- active segment append/read;
- terminal transaction records;
- atomic operation-head update;
- startup recovery;
- `avc init`, explicit `avc checkpoint`, `status`, `diff`, `log`, `show`;
- `fsck`; and
- crash injection at every write boundary.

**Exit criteria:** arbitrary injected crashes yield the exact pre-transaction or post-transaction state; deleting all caches and reopening preserves behavior.

### Phase 2 — Changes and operation recovery

Implement:

- stable `ChangeId`;
- editable current change;
- `new`, `describe`, `finish`/`commit`;
- replacement revisions;
- views with workspaces/checkpoints;
- `op log`, `undo`, `redo`, `op restore`; and
- operation retention/epoch skeleton.

**Exit criteria:** every tested history edit can be restored from the operation log; one million synthetic metadata-only operations remain queryable with bounded memory through indexing/pagination.

### Phase 3 — Workspaces, rewriting, and merge

Implement:

- multiple workspaces;
- split/fold/move/reorder/abandon;
- descendant rewriting;
- merge-base and deterministic tree/text merge;
- conflict objects and working-copy materialization;
- rebase and merge; and
- exact-ID rename detection.

**Exit criteria:** property tests against the reference model pass for randomized DAGs and operations; conflicted revisions can be saved, rebased, checked out, resolved, and restored.

### Phase 4 — Automatic savepoints and retention

Implement:

- dirstate;
- filesystem watcher abstraction;
- watcher-confidence state and recrawl;
- debounced daemon savepoints;
- explicit checkpoints;
- retention planner;
- mark/sweep reachability;
- segment compaction with reader leases; and
- `gc --explain`.

**Exit criteria:** watcher overflow cannot hide changes; retention keeps pinned/shared roots; crash-injected compaction never loses reachable objects; savepoint growth is bounded under the configured policy.

### Phase 5 — Remote, draft sync, and publication

Implement:

- bundle format;
- client/server capability negotiation;
- quarantine upload segment;
- hash/closure/quota validation;
- draft manifest CAS;
- public-view CAS publication;
- idempotency keys;
- event outbox; and
- `sync`, `fetch`, and `publish`.

A review `ProposalManifest` and `propose` command SHOULD follow once draft and publication semantics are stable; they are not required to prove the minimal remote.

**Exit criteria:** concurrent incompatible publications allow exactly one winner; retries do not duplicate state/events; rejected publications reuse previously uploaded objects; draft sync emits no public event by default.

### Phase 6 — Scale features chosen by evidence

Candidates:

- deterministic paged directory maps;
- revision graph/generation indexes;
- path-history index;
- selective large-file CDC;
- physical delta compression;
- lazy fetch and sparse materialization;
- structured merge drivers;
- reusable static clone bundles/CDN distribution, analogous in objective to Git bundle URIs ([Git bundle URI documentation](https://git-scm.com/docs/bundle-uri)); and
- optional virtual working copy.

Each feature requires a benchmark demonstrating the bottleneck and the improvement.

---

## 23. Incremental pull-request sequence

Keep each change buildable and useful. Create only the next few tasks rather than turning the entire sequence into process overhead.

1. **Repository skeleton and ADRs**
   Add workspace, lint/test/fuzz configuration, glossary, invariants, reference-model module, and format version constants.

2. **Canonical IDs and codec**
   Implement typed IDs, canonical primitives, object domain separation, parser limits, round-trip/noncanonical property tests, and golden vectors.

3. **Append-only segment store**
   Implement record framing, checksums, compression abstraction, active segment scan, object lookup, and short-write/truncation tests.

4. **Transactions and recovery**
   Implement begin/object/view/operation/commit records, atomic heads, deterministic recovery, `fsck`, and exhaustive injected-failure tests.

5. **Flat Merkle source tree**
   Implement file manifests, sorted flat directories, path update, tree lookup, diff, and snapshot construction. Keep a trait boundary for the large-directory implementation.

6. **Revisions, views, and operation log**
   Implement the explicit `checkpoint` command, log/show, operation history, restore, and disposable indexes.

7. **Stable changes and editing commands**
   Implement new/describe/finish, replacement revision semantics, undo/redo, and change-ID lookup.

8. **Working-copy status and checkout**
   Implement portable path validation, dirstate, safe file writes, pre/post read checks, status, and checkout recovery.

9. **Merge and conflicts**
   Implement merge base, tree merge, text merge, conflict terms, sidecar materialization, rebase, and randomized differential tests.

10. **Automatic savepoint daemon**
    Add watcher adapter, confidence state, debounce, explicit barrier API, retention planner, and daemon lifecycle.

11. **Compaction and GC**
    Add reachability, reader leases, immutable segment rewrite, atomic manifest switch, and fault tests.

12. **AVC remote**
    Add bundles, quarantine validation, draft CAS, publication CAS, idempotency, and event outbox.

Do not merge a PR that introduces an on-disk object without golden encoding vectors and malformed-input tests.

---

## 24. Open architecture decisions

### 24.1 Directory map

**Question:** Is a POS/prolly-style map materially better than flat canonical trees for AVC’s target repositories after accounting for object count and implementation cost?

**Decision method:** Phase 0 benchmark plus property proof of history-independent roots.

### 24.2 Hash algorithm

**Question:** BLAKE3-256, SHA-256, or another reviewed option as the default?

**Constraints:** speed, hardware support, security review, FIPS environments, library maturity, and long-term ecosystem support. The format remains hash-agile regardless.

### 24.3 Automatic-save retention

**Question:** Which time/count/storage policy gives useful recovery without surprising growth?

**Decision method:** collect anonymized/local opt-in edit traces or synthetic editor workloads; expose transparent policy and dry-run reporting.

### 24.4 Working-copy daemon

**Question:** daemon from Phase 1 or after explicit local semantics stabilize?

**Recommendation:** no daemon in Phase 1; add it in Phase 4. Repository operations must work correctly without it.

### 24.5 Large-file strategy

**Question:** whole blobs plus physical deltas versus CDC manifests, and at what workload-dependent threshold?

**Recommendation:** whole blobs first; schema supports chunked content; emit CDC only after evidence.

### 24.6 Operation-log concurrency

**Question:** serialize local writers permanently, or support Jujutsu-like divergent operation heads and automatic view merge?

**Recommendation:** serialize MVP writes, preserve DAG-compatible operation encoding, revisit after daemon/network-filesystem use cases are measured.

### 24.7 Public naming

**Question:** call shared moving names branches, bookmarks, or channels?

**Recommendation:** test terminology with users. Internally use `public_names`/`RefTarget`; do not couple workspace identity to public names.

### 24.8 Conflict algebra

**Question:** adopt full generalized merge terms in v1 or begin with a constrained three-way representation?

**Recommendation:** encode extensible normalized terms from the start; initially implement and thoroughly test common three-way behavior.

### 24.9 Import/export

**Question:** when should Git import/export be added?

**Recommendation:** only after AVC's local semantics are coherent. Provide one-time import and release/snapshot export before attempting bidirectional live synchronization.

---

## 25. Risks and scope controls

### 25.0 Alternative models considered

- **Patch algebra / Pijul-style change graphs.** Pijul’s change-centric model is valuable evidence that conflicts and independent changes can be represented differently from snapshot DAGs ([Pijul rationale](https://pijul.org/manual/why_pijul)). AVC does not adopt patch algebra as its v1 canonical model because implementation, explanation, partial-repository, binary, and operational evidence are weaker than for snapshots plus explicit conflict objects. It remains a research branch after the snapshot system works.
- **CRDT source files.** CRDTs can help replicate comments, labels, activity, and other collaboration metadata. Automatic convergence of character sequences does not establish semantic correctness of concurrent program edits, so source truth remains explicit revisions and merges.
- **Peer-to-peer publication.** Radicle is a useful current reference for local-first peer-to-peer collaboration and signed repository activity ([Radicle protocol](https://radicle.xyz/guides/protocol)). AVC keeps transport/topology separate from its object and publication model; a P2P transport may be added later.
- **Integrated all-in-one forge.** Fossil demonstrates the operational simplicity of a self-contained VCS, web UI, tickets, and wiki ([Fossil quick start](https://fossil-scm.org/home/doc/tip/www/quickstart.wiki)). AVC initially keeps source control small and exposes typed events/APIs rather than making every collaboration feature part of the trusted storage core.
- **Filesystem as the canonical VCS.** Copy-on-write filesystems and BranchFS-like mechanisms can accelerate workspaces, but AVC must remain correct on ordinary filesystems and across major desktop operating systems.

### 25.1 Principal risks

- Git’s content store may already be efficient enough that AVC’s storage gains are modest.
- Jujutsu may already solve most local UX problems with far greater ecosystem leverage.
- Automatic history may create privacy and retention problems larger than its recovery value.
- An AVC remote service may require substantial hosting infrastructure.
- Cross-platform working-copy correctness can dominate the project.
- Generalized conflict storage and descendant rewriting are difficult to explain and validate.
- A novel directory structure can increase object count and read amplification despite elegant asymptotics.
- Lack of Git/tooling interoperability can prevent adoption even by greenfield projects.
- “Quiet sync versus publish” only reduces platform load when the server and CI integrations preserve the distinction.

### 25.2 Engineering responses

- If the local model becomes difficult to explain, simplify commands and terminology without weakening recoverability.
- If automatic savepoints grow without bound or interrupt editing, tighten retention, batching, and background work.
- If a storage mechanism adds complexity without helping the working tool, keep the simpler interchangeable implementation.
- If collaboration appears to require a full forge, ship the smallest object and publication service needed for two repositories first.
- If conflict-bearing revisions become confusing or nondeterministic, constrain the first conflict algebra and keep deterministic textual fallback.
- If cross-platform paths or checkout behavior differ, adopt stricter portable rules and explicit errors rather than silent platform-specific outcomes.
- If Git interoperability is necessary for practical use, add bounded import/export without redefining AVC objects around Git.
- If publication batching does not reduce downstream events, fix the client/server event boundary rather than publishing private savepoints.

---

## 26. Features explicitly deferred

Do not build these before the core proof is complete:

- custom kernel filesystem;
- peer-to-peer synchronization;
- CRDT source files;
- LLM-based canonical merge;
- full semantic/AST repository storage;
- global code search;
- issue tracker/review UI;
- package and artifact registries;
- deployment system;
- global cross-tenant deduplication;
- bidirectional Git round-trip compatibility;
- unbounded automatic history;
- transparent recording of ignored/untracked files; and
- exhaustive benchmark breadth before core correctness.

---

## 27. Immediate implementation prompt

Use the following for the first working milestone:

> Replace the Git-backed command loop with the first useful AVC implementation slice in the existing Rust crate. Implement `avc init`, `avc checkpoint [name]`, `avc status`, `avc log`, and `avc restore <id>`. Store whole-file blobs by content ID, represent snapshots with deterministic flat directory trees, exclude `.avc/`, and keep physical storage deliberately simple and replaceable (loose objects plus an atomically replaced state file are sufficient). Preserve old snapshots, avoid rewriting user files except during explicit restore, and test snapshot/restore round trips plus interrupted state-file replacement. Do not implement automatic savepoints, logical changes, merge, multiple workspaces, a custom segment store, networking, or Git compatibility yet.

Once that loop works, use AVC on its own repository and continue with the next working capability—named changes and automatic savepoints—without attempting every later subsystem at once.

---

## 28. Annotated primary sources

### Existing VCS architecture and UX

- **Jujutsu architecture and operation model** — current documentation: [Architecture](https://jj-vcs.github.io/jj/latest/technical/architecture/), [operation log](https://jj-vcs.github.io/jj/latest/operation-log/), [concurrency](https://jj-vcs.github.io/jj/latest/technical/concurrency/), [working copy](https://jj-vcs.github.io/jj/latest/working-copy/), and [conflicts](https://jj-vcs.github.io/jj/latest/conflicts/). Relevant to stable changes, repository views, operation-level recovery, workspaces, and conflict-bearing revisions.
- **Sapling scale overview** — current documentation: [Working at scale](https://sapling-scm.com/docs/scale/overview/). Documents on-demand file/tree/commit fetching, filesystem monitoring, sparse profiles, incremental working-copy state, segmented graph techniques, and Mononoke.
- **EdenFS overview** — current source documentation: [EdenFS Overview](https://github.com/facebook/sapling/blob/main/eden/fs/docs/Overview.md). Relevant to lazy materialization, status/checkout acceleration, content hashes, notifications, and OS-specific virtual filesystem integration.
- **Mononoke public source caveats** — current source documentation: [Mononoke README](https://github.com/facebook/sapling/blob/main/eden/mononoke/README.md). Relevant to a Rust source-control server and the distinction between public code and a turnkey production stack.

### Persistent and multiversion data structures

- **ForkBase: An Efficient Storage Engine for Blockchain and Forkable Applications** — PVLDB 11(10), 2018: [paper](https://www.vldb.org/pvldb/vol11/p1137-wang.pdf). Introduces structurally invariant reusable indexes and POS-Tree, combining content-based slicing, Merkle hashing, and B+-tree ideas.
- **Dolt prolly trees** — engineering references: [Prolly chunker](https://www.dolthub.com/blog/2022-06-27-prolly-chunker/) and [millions of versions](https://www.dolthub.com/blog/2025-05-16-millions-of-versions/). Useful implementation evidence; vendor-reported and not a substitute for AVC benchmarks.
- **BetrFS/Bε-tree work** — [BetrFS project](https://www.betrfs.org/) and [FAST 2018 paper](https://oscarlab.github.io/papers/fast18-betrfs.pdf). Relevant to write-optimized derived indexes and the trade-off between full-path locality and rename complexity.

### Logging, filesystem hints, and synchronization

- **XLL: Cross-Layer Logging for Data Deduplication in Consensus-Based Storage** — NSDI 2026: [USENIX page](https://www.usenix.org/conference/nsdi26/presentation/shawger). Demonstrates eliminating duplicated durability writes in a TiKV prototype; informs AVC’s single append path.
- **SolFS: An Operation-Log Versioning File System for Hash-free Efficient Mobile Cloud Backup** — USENIX ATC 2025: [USENIX page](https://www.usenix.org/conference/atc25/presentation/pan). Relevant to per-file modified-range logs as synchronization hints.
- **SkySync: Accelerating File Synchronization with Collaborative Delta Generation** — FAST 2026: [USENIX page](https://www.usenix.org/conference/fast26/presentation/zhang-zhihao). Relevant to reusing storage-maintained checksums/digests rather than recomputing them in each layer.
- **Watchman troubleshooting** — current documentation: [recrawl and overflow behavior](https://facebook.github.io/watchman/docs/troubleshooting). Evidence that watcher queues can lose synchronization and require full reconciliation.
- **Practical Rateless Set Reconciliation** — ACM SIGCOMM 2024: [paper](https://dl.acm.org/doi/10.1145/3651890.3672219). Future option for reconciling large overlapping object sets when difference size is unknown.

### Deduplication and large files

- **VectorCDC: Accelerating Data Deduplication with Vector Instructions** — FAST 2025: [USENIX page](https://www.usenix.org/conference/fast25/presentation/udayashankar). Demonstrates that CDC scanning is a meaningful cost and can be vectorized.
- **Breaking and Fixing Content-Defined Chunking** — IACR ePrint 2025/558: [preprint](https://eprint.iacr.org/2025/558). Relevant to keyed-CDC security and encrypted deduplication threat models.

### Workspaces and merge

- **Fork, Explore, Commit: OS Primitives for Agentic Exploration / BranchFS** — February 2026 preprint: [paper](https://arxiv.org/abs/2602.08199), [source](https://github.com/multikernel/branchfs). Relevant to optional future copy-on-write workspace branches; preliminary rather than a production baseline.
- **Mergiraf** — current project documentation: [architecture](https://mergiraf.org/architecture.html). Relevant to line-first, syntax-aware structured merge with fallback.

### Build/storage separation and Git lessons

- **Bazel remote caching** — current official documentation: [remote caching](https://bazel.build/remote/caching). Separates action-result metadata from a content-addressable output store.
- **Git hash transition** — current official documentation: [hash-function-transition](https://git-scm.com/docs/hash-function-transition). Demonstrates migration complexity when object-ID assumptions are pervasive.
- **Git bundle URI** — current official documentation: [bundle-uri](https://git-scm.com/docs/bundle-uri). Relevant to serving reusable object bundles through static/CDN infrastructure.
- **Git commit graph, multi-pack index, and reachability indexes** — current official documentation: [Git configuration](https://git-scm.com/docs/git-config) and [user manual](https://git-scm.com/docs/user-manual). Relevant to keeping graph and physical-location indexes derived from immutable objects.
- **Multihash** — current specification overview: [multihash](https://multiformats.io/multihash/). Relevant to self-describing hash identifiers and algorithm agility.

### Alternative version-control and collaboration models

- **Pijul** — current manual: [Why Pijul](https://pijul.org/manual/why_pijul). Relevant to change-centric history and first-class conflict relationships.
- **Radicle** — current guide: [protocol](https://radicle.xyz/guides/protocol). Relevant to local-first peer-to-peer collaboration and signed repository activity, while retaining Git underneath.
- **Fossil** — current documentation: [quick start](https://fossil-scm.org/home/doc/tip/www/quickstart.wiki). Relevant to a self-contained VCS and collaboration product with a small operational footprint.

### Correctness

- **Metis: File System Model Checking via Versatile Input and State Exploration** — FAST 2024: [USENIX page](https://www.usenix.org/conference/fast24/presentation/liu-yifei). Relevant to differential reference models, state exploration, and reproducible storage bug discovery.

---

## 29. Final recommendation

Build AVC as a **journaled snapshot VCS that provides automatic recovery, intentional history, and atomic publication**.

The core combination is:

```text
immutable snapshots
+ stable logical changes
+ a separate operation/view graph
+ automatic bounded savepoints
+ first-class conflicts and workspaces
+ one append-only physical write path
+ deterministic persistent directory maps
+ derived rebuildable indexes
+ quiet draft synchronization
+ atomic explicit publication
```

The most important proof is not that AVC can create revisions faster than Git. It is that AVC can protect work continuously, keep local and shared history understandable, recover every operation safely, scale with changed data, synchronize without unnecessary remote events, and remain correct under crashes and untrusted input.
