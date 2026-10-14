# AVC roadmap: prove the native model before stabilizing it

**Updated:** 2026-10-14  
**Architecture:** [design.md](design.md)  
**Evidence and alternatives:** [research.md](research.md)

The Rust 0.0.1 implementation is still a Git-command wrapper with a separate JSON operation log, not the proposed native VCS. Do not layer new safety promises over the old Git reset/squash flow. Replace it incrementally only once a native vertical slice actually works. Detect and preserve existing .avc repositories; do not silently interpret the legacy format as a native one.

**Delivery objective:** the smallest coherent local VCS that proves authored work can be composed against changing bases **without rewriting the original authored version**, while exact source states, recovery and concurrent workspace roots remain safe. Do not start by building a distributed forge, agent scheduler, full virtual filesystem or a generic framework of untested storage backends.

## Phase 0 — Specify competing complete models

### A. Executable state machine and oracle

Model the immutable source State, stable Work identity, WorkVersion variants, Application with explicit base/result, typed conflicts, Workspace roots/generations, recovery captures and shared targets.

Operations: create work, author version, create divergent version, declare dependency, apply one or several works, resolve conflict, advance an independent workspace, undo workspace operation, capture/expire recovery, propose, and conditionally publish. Write a **small independent oracle** (not production types feeding themselves) with exhaustive or property-based traces for short histories.

Required counterexamples:

- Two workers from the same base, disjoint edits and overlapping edits.
- Stale input dependency even when edited paths are disjoint.
- A depends on B-v1, but target has B-v2.
- Two divergent authored versions with the same WorkId; explicit version selection or resolution instead of last-writer-wins.
- Duplicate application of a previously accepted WorkVersion: integration provenance detects the repeat even when source bytes look identical.
- Declared exact dependency A-v1 is not silently considered satisfied by A-v2; authorized substitution must be recorded.
- Conflict reappearance and exact-context resolution reuse.
- Offline divergent target roots and later synchronization.
- Split/fold and revert, preserving authored provenance.
- Path rename versus edit, delete/recreate, binary replacement and directory/type collisions.
- Lost response after committed root transition and duplicate request.
- Capture interrupted by concurrent filesystem edits; attempt to restore after further edits.

**Decision gate:** the leading model must give unambiguous outcomes, no source-loss path, and a simple human interface. A hidden second authority or unspecifiable “same semantic intent” breaks the design.

### B. Real source-model competition

Implement **small, comparable prototypes** on identical source fixtures:

1. Immutable persistent tree State + authored WorkVersion + Application, with verified changed-path/precondition effects.
2. A true patch/change graph with an implementation of commutation and conflict/dependency laws—not a collection of normal text diffs branded as patches.
3. A canonical changed-file-log model with derived/rebuilt tree snapshots, testing arbitrary offline historical reads and reconstruction.

Use the same opaque-file, rename, repeated-line, binary, multiway conflict and independent-worker traces. Compare identities under reordering, conflict quality, total bytes, CPU/memory, tree reconstruction latency and history evolution. Treat ambiguous transformations as conflicts rather than adjusting expected results to make the prototype pass.

**Decision gate:** choose and specify **one authoritative source model**. Keep the snapshot/WorkVersion/Application model only if it actually wins the chosen correctness, generality and complete-workload criteria.

### C. Tree and local durability competition

Tree structures: sorted flat directory baselines; deterministic paged Merkle maps; history-independent immutable search trees. Sweep realistic trees plus directories of 10, 1,000, 100,000 and 1,000,000 entries, adversarial sorted/colliding names, rename/move, one-path updates, sparse diffs and page-boundary shifts. Require canonical equal-content/equal-root property and bounded pathological behavior.

Physical stores must implement the **same** object and resource-root transitions:

- one append-only object/transaction journal with derived indexes;
- immutable segments plus SQLite authoritative root transactions;
- SQLite small objects/roots plus external large objects.

Measure explicit durability barriers, read latency, foreground p50/p95/p99, sync/write amplification, 1/8/32 writers, GC/compaction, startup recovery, saved-space retention and consistent backup/recovery. Do not extrapolate from unrelated KV-storage benchmarks. Test correct object admission against concurrent collection, reader leases, segment replacement, and directory-entry persistence.

**Decision gate:** select the simplest engine passing the strictest safety invariants and representative workload performance. Document assumptions for APFS, ext4/Btrfs and NTFS as supported; process-kill tests do not prove power-loss durability.

### D. Git compatibility canary

Before stabilizing formats, use a **real Git fixture** with ASCII and non-UTF8 paths, modes, symlinks, rename, merges, signed objects, attributes/EOL, LFS/submodules, and a shallow/partial example. Identify which forms must be preserved and what cannot round-trip. Declare errors for unsupported subsets. This is a design feasibility experiment, not complete interchange implementation.

## Phase 1 — Native local vertical slice

Implement in Rust, starting with cohesive source-object, repository-transaction, working-copy, and CLI responsibilities in the existing crate. Split into more crates when actual boundaries justify it.

Working user loop (command spellings are provisional):

~~~text
avc init
avc status
avc change new "Add parser"
edit files normally
avc change finish
avc history
avc undo
avc restore <checkpoint>
~~~

Behind it: immutable States; authored WorkVersions with exact base/result; at least simple Applications; independently versioned WorkspaceRoot; durable operation events for undo; explicit source/checkout separation; status and diff over current bytes; format version detection.

**Gate:** safe reopen and restore after injected failure at every object/commit/materialization boundary; no acknowledged State with missing closure; no silent loss of dirty editor bytes; exact source restoration after a failed operation. Ordinary small-repo responsiveness without network or daemon.

## Phase 2 — Independent work and recovery

Implement multiple isolated workspaces over one shared immutable store; group application of authored work against a chosen State, exact preconditions and dependency closure, deterministic merges with typed conflicts, explicit divergence, optional move provenance and structure-aware drivers only where proven.

Add private auto-capture and named pins: tracked/admitted files, watcher hints with verification/reconciliation, bounded time/count/size retention, visible last durable capture, GC with staging/reader protection and explicit protection-status errors. Captures must not create permanent authored revisions or public events.

Expose **the same semantic API without requiring a checkout**: inspect, diff, update a State, create/evolve WorkVersion, apply/compose, resolve conflict, list revision/operation history, and recover. Schema-versioned machine requests use exact expected IDs, workspace generations, bounded output and idempotency epochs. A CLI, an editor and an agent library call the same core.

**Gate:** 1/8/32 independent workers; no spurious logical CAS contention on a global head; shared-actor/different-session isolation; divergent changes remain visible; original authored versions survive reapplication to new bases; no accidental conflict-resolution reuse. Reproduce Trellis-style stale-materialization clobber scenarios. Prove recovery retention actually frees bytes.

## Phase 3 — Git import/export and external collaboration

Import source and history with original-ID mapping; export selected **resolved** native States/accepted applications to genuine Git refs and commits, with declared native metadata loss. Work through at least one ordinary Git host's existing review and merge protection, without creating a dual-writer checkout/index.

A compatibility matrix must specify supported and rejected cases for: paths/Unicode/casefold, executable modes, symlinks, ignore rules, attributes/filters/EOL, tags/signatures, submodules, LFS, binary files and shallow/partial history. All unsupported mutations must fail before corrupting or silently changing source. Verify conversion by exact byte/mode fixtures, not just matching numbers of commits.

**Gate:** a native user can collaborate with an ordinary Git user on a qualified repository, with no private recovery roots pushed; host timeouts and unknown outcomes can be reconciled. Compare the experience with current Git worktrees and Jujutsu, including changed-base reapplication and exact-candidate validation. If the native benefit is marginal, revisit the product model, not just the CLI.

## Phase 4 — Optional native sync and accepted integration

Implement resumable batched object/State fetch and push, root negotiation, quarantine/hash/closure checks, authorized restricted draft sync and independent public targets. No always-online requirement for ordinary source editing.

Add immutable composed Candidates, versioned validation policy/recipe, evidence and acceptance referencing exact candidate IDs, and per-target conditional publication/receipt/outbox. Shared-target tests run **outside** publication locks. Requests with stale target, changed policy, revoked actor, missing evidence or incompatible dependency never silently land. Group application can reduce the number of target transitions, but the final group State still needs required validation.

**Gate:** network partition, retries, target drift during CI, wrong evidence digest, changed toolchain/recipe, post-commit lost responses, at-least-once duplicate events and quota/authorization failures. Strong guarantees apply only where AVC owns target authority; an external Git host retains its own rules.

## Phase 5 — Scale selected by evidence

Candidates include canonical paged wide directories, selective file-content chunking, bounded delta compression, sparse/virtual materialization, recursive graph/path/blame indexes, cache tiers, syntax/symbol-derived merge support and per-path publication leases for nonmergeable assets.

Each feature needs a measured limiting workload and a simpler tuned baseline. Evaluate full lifecycle cost: read amplification, memory, storage, compaction, cold cache, recovery, Git bridge, synchronization and CI/notification events. Do not impose fleet-only complexity on a five-person project.

## Continuous quality gates and comparison workloads

| Question | Evidence |
| --- | --- |
| Can the source be reconstructed? | Canonical encoding vectors, independent reference States, malformed-object fuzzing and model traces |
| Can acknowledged work be lost? | Crash/power-loss-specific fault testing at every persistence barrier, GC/reader interleavings and consistent backup/restore |
| Can a restore clobber newer edits? | Concurrent external editor, watcher overflow, interrupted materialization and stale workspace tests |
| Can independent workers progress? | 1/8/32 worker contention/conflict benchmarks, measured logical vs physical serialization |
| Does change identity survive context shifts? | Authored WorkVersion reused in two Applications, dependency rewrite, divergence and revert traces |
| Does composition preserve intent? | Deterministic outcomes, typed conflicts, exact preconditions; semantic-interference scenarios with CI |
| Is Git exchange trustworthy? | Qualified round-trip bytes/modes/metadata, sign/attr/LFS/submodule failures explicit |
| Is the product better? | Complete solo and agent tasks vs maintained Git/Jujutsu, measuring user time, retries, bytes, latency, storage and CI cost |

Reference workloads: typical small/medium projects, large monorepos, wide directories, long histories, 100,000 named revisions, 1,000,000 bounded private captures, binary edits, overlapping/nonoverlapping agent proposals, read-heavy no-checkout queries, and batched validated publication.

Record hardware, filesystem, pinned source versions, exact commands/configs, fixed random seeds, cold/warm cache and raw results. Upstream published speedups (e.g. Scion) are not AVC benchmarks. Correctness failures block release regardless of speed. Experimental users need independent backups and explicit usable export until the full safety and compatibility contracts are qualified.

Existing Rust prototype checks remain:

~~~sh
cargo test
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
~~~

No Rust implementation change is implied by this design update, and those checks alone cannot verify native VCS guarantees.
