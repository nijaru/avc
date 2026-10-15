# AVC roadmap: validate source authority, then build the native VCS

**Updated:** 2026-10-14  
**Architecture:** [design](design.md) · **evidence:** [research](research.md)  
**Current code:** legacy Rust 0.0.1 Git-command wrapper with separately written JSON operation log; no native guarantees.

The new reference is an **immutable snapshot-root State DAG**. This supersedes the old decision to require canonical first-parent changed-file records. Both are real alternatives, but source-root states now lead for a general-purpose offline/local-to-cloud VCS. This is a testable selection, not an asserted benchmark result. Do not implement obsolete WorkVersion/Application scaffolding, a "Bonsai clone", or Enroute-style global dense IDs.

**Git compatibility, Git object/pack formats and Git migration are not requirements or initial release gates.** Evaluate adapters later if users need them.

## Phase 0 — executable source-state semantics and architecture comparison

Before stabilizing object IDs or writing a new persistent store, implement a small pure **reference interpreter** over logical directory maps, separate from Rust production objects:

Add a separate small **workspace/capture/materialization state-machine oracle** before building the CLI. A recorded selected revision is not the bytes in an uncontrolled directory. Keep workspace selection generation, capture-head generation and materialization progress independent; persist observed capture coverage, not a forever-clean bit. Treat conflict *value* identity and conflict *origin* as independently modeled types.

    State = exact typed tree
    build(base, expected-path edits) -> State
    root(State) -> canonical digest under chosen format
    Revision = {StateId, parent revision IDs, LogicalChangeId, metadata}
    merge(base, a, b, driver) -> State with typed conflicts
    amend/rebase -> new immutable revision + predecessor relation

Build a narrow snapshot-root prototype and a first-parent changed-file-history prototype over the **same** logical state, blobs, tree codec, merge algorithm, and test fixtures. Do not write two complete CLIs or network services. A causal patch graph gets a faithful semantics-specific experiment only if concrete workflows justify its extra authority model.

### Correctness matrix

- Empty files/directories, symlinks, modes, NUL/non-UTF8 content, invalid path components and portable name collisions.
- Different authors and ancestry producing equal source: equal StateId in one canonical format, different RevisionIds.
- Change amend/rebase divergence; split/fold lineage; legitimate revert and later reapplication.
- Directory subtree copy/move; rename followed by edit; delete-versus-modify; file/directory type conflict; copy from an independently retained subtree root.
- Merge with multiple bases, n-way unresolved conflicts, exact base/left/right recorded provenance; no hidden random ordering.
- Source reconstruction with every derived index deleted; no missing referenced objects; corrupted hash rejected.
- Source operation and revision object consistency across differing ingestion/update orders.
- Untrusted giant input, path traversal, unsupported metadata, malicious symlinks and invalid file kinds.
- Crash-before-root, crash-after-root, retry after lost reply, GC while objects are staged, reader during compaction.
- Concurrent distinct workspace root updates, repeated CAS to one target, stale in-place filesystem restore attempt.

Keep an **independent** oracle for source bytes and roots. Property-based and state-machine testing should assert against the independently specified contract, not compare two implementations of one shared bug.

### P0 adversarial lifecycle traces

These are **format and safety blockers**, not cosmetic tests:

1. Editor modifies an ordinary directory after AVC verified it: status must not preserve a perpetual clean guarantee; switching cannot overwrite those unobserved bytes under a no-clobber claim.
2. Watcher observes A but not B, drops events, or dies mid-capture: coverage and acknowledgement disclose exactly what was preserved. An interrupted capture does not replace the last durable pin.
3. Editor modifies two files while they are scanned: a per-file observation can preserve its bytes, but cannot claim the pair existed simultaneously unless writer exclusion/snapshot was demonstrated.
4. Frequent capture-head updates race an independent workspace selection CAS: selection must not spuriously fail on capture generation changes.
5. Selected Revision changes during a scan: preserve an acknowledged recovery observation against its original base; never silently retarget the observed bytes onto the new selected Revision.
6. Conflict value has identical base/alternatives but different author/revision/driver origins: both source States have equal IDs; their immutable ConflictOrigin records remain distinct. Different unresolved semantic base terms must remain distinguishable.
7. Delete/edit directory collision, merge of already conflicted states, three-way and criss-cross/n-way ancestry: preserve distinct alternatives and prohibit silently flattening nested conflict provenance.
8. Rebase single-parent change P->C onto Q: produce exact new source (or typed conflict), keep old C, record evolution; reject unsupported merge-commit rebases. Test revert-after-duplicate-change and intentional reapply.
9. A sparse client lacks a complete base: service verifies path *including absence* before constructing StateId. Offline client with missing witnesses retains a pending edit bundle, not an invented authenticated complete source root.
10. Agent uploads new blobs and terminates without a pinned draft root: no durable-handoff guarantee. After an acknowledged root and full object closure, destroying the VM leaves the revision reconstructible through a second client.
11. Crash after root CAS but before directory materialization: repository source remains durable, checkout explicitly incomplete, and prior directory remains intact. Lost RPC response is resolved by request identity.
12. No path traversal or symlink construction escapes a fresh materialization destination. Case-colliding paths cannot be silently renamed or dropped.

Use model-generated short interleavings and small independent fixtures, then real filesystem tests for claims involving uncontrolled writers; a simulated clean flag is not filesystem evidence.

### Representative work, not isolated microbenchmarks

| Workload | Comparison required |
| --- | --- |
| Solo edit-test-repeat | Write + first source read + diff + next edit + restore |
| High-frequency recovery captures | Durable admitted bytes + bounded storage + complete reclaim |
| Many durable but discarded draft revisions | Admission, cold first read if later requested, retention cleanup |
| Many durable revisions immediately built | Admission + source-tree retrieval + build materialization |
| 100k+ history with caches removed | Cold arbitrary source, path history, index rebuild, tail latency |
| Many-file moves and 1M-entry wide directories | Node allocation, source read, subtree reuse, diff, remote transfer |
| 8/32 isolated workers | Aggregate writes, resource-root conflicts, shared cache, tested integration |
| Large binaries and source distribution | Whole-blob, bounded delta/chunk and sparse/bulk network bytes |
| Unsafe process/fs/storage interruptions | Reachability, data-loss outcomes, restoration, consistent backup |

Compare against appropriately configured modern Git and Jujutsu for *complete user tasks*. Scion/Mononoke/Enroute published fleet results are not comparable AVC numbers. Record hardware, filesystem, durability level, real or seeded source histories, warm/cold cache, p50/p95/p99, CPU, disk/network bytes and errors.

**Canonical history gate:** snapshot-first remains selected unless a parent-relative authoritative log demonstrates materially better end-to-end results without unacceptable arbitrary-version read/recovery costs. For causal patch graphs, show useful selective-undo/conflict gains on ordinary external file-writer histories, not just synthetic commutative examples.

**Canonical tree layout gate:** test sorted small directories against deterministic bounded radix/sharded and paged/POS directory nodes before freezing StateId encoding. Index format migration affects canonical hashes; document old decoder and explicit object remapping.

## Phase 1 — safe native local vertical slice

Replace the old Git-dependent code only when a native path is operational. Implement compact, cohesive Rust components:

1. Typed content objects, deterministic canonical encoding/hash vectors, path validation and immutable source trees.
2. Revision creation, ancestry and stable logical change identity, without committing a change-diff as an independent source authority.
3. SQLite reference store for source objects and resource-root/operation transactions with explicit durability assumptions.
4. Working directory status reporting selected Revision, last verified observation, latest durable CaptureId, completeness/consistency class, and pending materialization **as separate facts**; tracked/admitted file capture (no implicit secret/build-output inclusion).
5. Checkout-free source read/list/diff/history with typed unresolved-conflict responses (never ordinary marker bytes), and safe restoration into a **new** directory.
6. Independent workspace selection and capture-head generations plus durable materialization intents; one short transaction records the affected resource roots, previous values, pins, idempotent request outcomes and new head.
7. Native self-contained source/history export with verification and backup/restore.

Use provisional intuitive commands: init, status, capture, change finish, log, diff, show, new workspace, restore --into, verify/export. Command spelling must not determine history semantics.

**Acceptance:** no Git executable/server required; exact source reconstruction after reopen; deterministic source root equality even when conflict provenance differs; durable root/object/capture closure; no false perpetual clean status; old workspace and last capture preserved across failed scans; clean handling of partial/corrupt input and disk full; no destructive filesystem restore without writer exclusion.

Detect existing legacy .avc metadata and preserve/offer explicit migration or export. Never reinterpret the Git-wrapper format as native or delete user data because the old prototype is v0.

## Phase 2 — recoverable history and composition

Implement meaningful source history operations (amend, new, rebase, merge, undo, revert, split/fold as semantics justify), with typed first-class conflicts. Preserve original revisions under explicit retention and expose divergence instead of picking a timestamp winner. Automatic/explicit recovery captures have bounded separate retention, named pins and honest durable coverage.

Write the conservative three-way/multi-base merger and explicit structural conflict cases. Canonical ConflictValue must exclude author/history provenance; bind origin records to exact revision and path occurrence, and fail format stabilization until multiway normalization and conflict-of-conflicts are specified and tested. Benchmarks must separately report unnecessary conflicts and incorrect automatic merges. Syntax/token drivers and AI-proposed resolutions are optional and never canonical authorities.

Keep filesystem source state and the working-copy materialization journal separate. Test watcher overflow, external editor races, stale process file descriptors, incomplete restore and unsafe symlink paths. In uncontrolled directories, **guaranteed non-clobber recovery is into a new location**.

**Acceptance:** the same source/merge operations are callable without checkout; typed file/directory conflicts survive state reconstruction; private captures neither create public events nor keep every old capture alive through an immortal history graph. An exact source revision is always recoverable after an operation fails between metadata and filesystem updates.

## Phase 3 — native transport and ephemeral-workspace end-to-end scenario

Define a versioned native protocol independently of Git:

- Batch/resume immutable object and subtree retrieval, including expected hashes, ranges, promised-object errors and backpressure.
- Transfer a complete selected source and optionally selected history; make offline copies useful after complete acquisition.
- Headless read/diff/merge/author operations on exact IDs without a full working copy.
- Per-target conditional updates, separate private/draft roots and explicit authenticated permission scope.

**Mandatory system scenario:**

1. A developer chooses an immutable revision on a laptop or VPS.
2. A short-lived microVM/container obtains a sparse or full source view through shared cache or bulk transfer.
3. An agent uses ordinary files and records a native immutable revision.
4. Its required object closure **and a private, owner-scoped draft root/pin with explicit retention** become durable before a DraftReceipt is acknowledged and its runtime is deleted.
5. Another machine reconstructs exactly that source with no dependency on the original VM disk.
6. Multiple useful revisions are combined into an exact candidate; conflicts are retained or deliberately resolved; target advancement is conditional on current policy.

Test disconnection/offline edit, duplicate/reordered upload, partial missing objects and absent-path witnesses, stale target, unauthorized read, network timeouts after commit, upload-without-root-pinning, root expiry, and cold/busy caches. Do not claim remote operations are atomic with an external CI/deployment system.

## Phase 4 — choose physical optimization based on measurements

Start with a single local SQLite authority and whole blobs. Introduce only measured optimizations:

- Immutable segment files with crash-safe staging pins, object/GC coordination and consistent backups.
- Paged/bounded canonical directory nodes selected in Phase 0; independent derived lookup/index formats.
- Ancestry, changed-path, blame and optional search indexes keyed by immutable revisions + index version.
- Native sparse-object queries **and** compressed bulk bundles; select by data/latency needs.
- Physical delta chains/content-defined chunking for large changing blobs under maximum reconstruction bounds.
- Shared service caches and source/merge compute isolation if hosted demand justifies them.

Do not require Enroute's stable globally numbered objects, Raft, pack format or commit segments. They solve particular hosted Git-serving workloads, not universal source-history semantics. Do not require Scion's always-online clients or its specific changeset protocol.

## Phase 5 — publication and trustworthy release

A candidate refers to an exact result StateId, selected revisions, merge algorithm/recipe, target generation and validation policy. Evidence binds to that candidate and toolchain. A protected target moves only after validation under current policy and authorization; alternative accepted/verified staging is an explicit weaker policy.

Finish real fault-injection tests on supported APFS/Linux/NTFS configurations and actual durable-flush assumptions, backups, garbage collection with active readers/staging, format robustness and malicious content. Audit encryption/authentication for service use, not just content hashes. Make no claim of production data integrity from passing unit tests alone.

The native export/backup/recovery experience must be usable and verified before asking people to put irreplaceable source in AVC.

## Optional future adapters and naming

Evaluate Git import/export, other VCS interoperability or Git physical codec **after** native correctness/performance goals justify the extra surface. No compatibility requirement dictates native formats or delays the first release. Project naming is independent; keep AVC until selected.

Existing prototype checks remain:

    cargo test
    cargo fmt --check
    cargo clippy --all-targets --all-features -- -D warnings

The [existing narrow Python probes](experiments/vcs_contract_probes.py) and [results](experiments/vcs_contract_results.json) remain evidence of specific hazards, not a native benchmark. No Rust implementation change, measurement or release is implied by this architecture documentation update.
