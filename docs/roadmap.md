# AVC native implementation roadmap

**Decision:** native source and storage engine, local-first; Git import/export boundary.  
**Architecture:** [design.md](design.md)  
**Evidence:** [research.md](research.md)

The current Rust 0.0.1 source is a Git wrapper. Its Git command helpers, JSON operation log and automatic Git commit/squash flow do not provide the planned atomicity. Replace affected code and tests as vertical slices land. Keep Git history as the old implementation record; no permanent legacy engine. Detect old .avc repositories and offer preservation/import rather than silently reinterpreting their format.

## Phase 0 — Format and safety prototypes

Specify canonical encodings, object IDs, operation/view transitions, root authority, retention pins and idempotency. Build an executable reference model independent of production code. Benchmark a few directory maps (flat, deterministic paged Merkle, immutable search tree) on realistic and adversarial inputs, including different insertion orders leading to identical roots.

Prototype immutable append segments with one SQLite metadata root transaction; test data durability barriers, roots, reader leases, GC and interruption. Review SQLite/filesystem guarantees rather than infer them from process-kill tests. Choose the simplest correct initial physical representation.

**Gate:** fixed encoding vectors; repeatable root/transaction crash tests; no acknowledged root with missing objects; documented supported filesystem assumptions.

## Phase 1 — Real native local VCS loop

Implement one Rust library and CLI vertical slice: initialize/open, immutable whole-file content, hierarchical trees, snapshot, status, diff, log, stable change/revision identity, operation history, explicit checkpoint, undo/restore, durable root publication and safe materialization. One authority owns roots; no Git or jj engine underneath.

Start with small flat tree objects and whole blobs; retain later algorithm choices behind cohesive object interfaces rather than speculative backend frameworks. Use versioned on-disk format and explicit error paths. No required daemon, hosted service or custom filesystem.

**Gate:** crash injection at every object/metadata/checkout boundary, full reopen/recovery, no silent destruction of edited files. Fuzz malformed objects, names and indexes.

## Phase 2 — Concurrent workspaces and bounded recovery

Implement separate mutable directories sharing immutable store, CAS on workspace generations, divergent change revisions, split/fold/rebase, persistent typed conflicts and deterministic merges. Implement watcher-assisted auto-capture for admitted files, explicit protection status, coalescing, retention budgets, safe GC and explicit pinning.

Add a typed checkout-free library API with exact edit/file preconditions, versioned requests, structured conflicts, bounded streaming queries and durable idempotent outcomes. GUI/MCP/agent adaptors must use the same semantics, not shell scripts as the only truth.

**Gate:** 1/8/32 writers, stale generations, concurrent editors, watcher overflow, crash/GC interleavings, long-running readers, million-event bounded recovery traces. Correctness and retention first, throughput second.

## Phase 3 — Git exchange and early interoperability

Import real Git source/history while retaining mappings to original objects. Export supported resolved native revisions as valid Git history. Integrate with one existing Git hosting path for proposals and reviews, honoring **host** authority for protected branches and CI. Do not silently share a Git index or worktree with native writers.

Maintain tested feature matrix: modes, symlinks, ignores, attributes/EOL/filters, tags/signatures, submodules, LFS, partial/shallow history and path encoding. Unsupported behavior must error before data loss, not be called compatible.

**Gate:** native/Git user collaboration, fixture round-trip for declared supported subset, no leakage of private recovery objects, explicit unknown-network-result reconciliation.

At this stage compare complete Git+worktrees, Jujutsu, and AVC workflows. Native development continues only if it demonstrates meaningful user value beyond a CLI variation.

## Phase 4 — Native collaboration and validated landing

Build optional bounded native object sync and server: quarantine, closure/hash/authorization checks, restricted draft roots, proposal records, immutable combined candidates, exact test/approval evidence, target CAS, policy epochs, receipts and event outbox. No required forge, agent scheduler or CI runner. Existing host adapters retain their weaker atomicity semantics.

**Gate:** target advances/revocations during validation, nonoverlapping edits that break an API together, duplicate requests, cancellation, network partition and repeated event delivery. Only an exact validated candidate can satisfy required policy.

## Phase 5 — Optimization based on evidence

Consider canonical paged wide directories, selective content-defined chunking, compressed segment packing, lazy/virtual working copies, scalable history/blame indexes, object-cache tiers, bounded merge drivers and asset publication locks. Require full-workload wins before adopting any technique.

## Benchmark and assurance program

Compare maintained/tuned Git, current Jujutsu and native AVC with pinned source versions. Use ordinary projects and controlled large trees. Test clean/dirty status, edits to same/different files, 100k commits, up to 1m recovery snapshots, wide directories, binary churn, cold/warm history queries, 1/8/32 concurrent workers, sparse materialization, imports/exports, candidate integration and CI/event amplification.

Record latency distributions, CPU/RAM, bytes, object counts, GC, network, recovery time, stale/conflict rates, full workflow completion and failure behavior. Publish hardware, filesystem, command/config versions, seeds and raw results. Never apply Scion's author-reported fleet speedups to AVC as predicted gains.

Model-check core transitions; fuzz untrusted input; run crash/fault injection around every durability barrier, ref/metadata CAS, checkout replacement, GC and event acknowledgment. Qualify supported filesystem/OS behavior separately. Distinguish process termination from power loss. Independent backups and usable Git exports remain required during experimental development.

Current Rust checks remain:

    cargo test
    cargo fmt --check
    cargo clippy --all-targets --all-features -- -D warnings

Passing them does not establish the new contracts. Do not label native AVC safe for irreplaceable work until integrity, GC, restore and compatibility gates have been independently satisfied.
