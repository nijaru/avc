# AVC native implementation roadmap

**Decision:** native source and storage engine, local-first; Git import/export boundary.  
**Architecture:** [design.md](design.md)  
**Evidence:** [research.md](research.md)

The current Rust 0.0.1 source is a Git wrapper. Its Git command helpers, JSON operation log and automatic Git commit/squash flow do not provide the planned atomicity. Replace affected code and tests as vertical slices land. Keep Git history as the old implementation record; no permanent legacy engine. Detect old .avc repositories and offer preservation/import rather than silently reinterpreting their format.

## Phase 0 — Format and safety prototypes

Do **three targeted experiments before finalizing the native format**, rather than treating the previous mechanism choices as validated.

1. **Source/change model:** independently specify revisions, ChangeId lineage, operations, conflicts, recovery, and per-workspace/target CAS. Build an executable reference state machine. Prototype snapshot+changed-path composition and a small genuine patch-commutation alternative; compare exact historic reconstruction, mixed binaries, renames, conflict reapplication, undo/rebase and Git translation. Patch-algebra probes must exercise inversion and round-trip commutation laws, not just text-diff replay.
2. **Persistent directories:** benchmark flat canonical trees, deterministic paged Merkle maps and a canonical immutable search-tree baseline. Use ordinary repositories plus 10, 1,000, 100,000 and 1,000,000-entry directories, adversarial path distributions, update-order invariance, move operations and boundary-shift merges.
3. **Durable store:** implement minimal complete root transactions in (a) a single append-only object/root journal, (b) immutable segments plus authoritative SQLite metadata and (c) SQLite small objects/metadata plus separately persisted large objects. All three must preserve the same object identity and transition model. Measure synchronous write performance, reopen, retained space, reader contention, checkpoint/compaction, and GC with process-kill/fault injection.

Protect prepared objects from GC before publication, including concurrent compaction. Validate that independent workspace head advancements do not logically contend on one global root, even if the local journal/database serializes physical commits briefly. Document SQLite WAL synchronous configuration and real filesystem durability assumptions.

**Gate:** fixed canonical encoding vectors; repeatable reference-state and crash tests; independent workspace CAS correctness; no acknowledged root with missing objects; consistent backup/repair model; one selected physical engine *with evidence*. Choosing the most fashionable storage algorithm is not an acceptable result.

## Phase 1 — Real native local VCS loop

Implement one Rust library and CLI vertical slice: initialize/open, immutable whole-file content, hierarchical trees, snapshot, status, diff, log, stable change/revision identity, operation history, explicit checkpoint, undo/restore, durable root publication and safe materialization. One authority owns roots; no Git or jj engine underneath.

Start with small flat tree objects and whole blobs; retain later algorithm choices behind cohesive object interfaces rather than speculative backend frameworks. Use versioned on-disk format and explicit error paths. No required daemon, hosted service or custom filesystem.

**Gate:** crash injection at every object/metadata/checkout boundary, full reopen/recovery, no silent destruction of edited files. Fuzz malformed objects, names and indexes. Reconstruct arbitrary revisions without replaying a growing log. Verify two independent workspaces can commit without moving a single shared logical head. Reproduce a Trellis-like failure case: a separate editor modifies files after an observer's scan but before a later materialization; neither restore, journal catch-up nor sync may overwrite those unseen changes. Distinguish committed source state from successfully updated filesystem state.

## Phase 2 — Concurrent workspaces and bounded recovery

Implement separate mutable directories sharing immutable store, CAS on workspace generations, divergent change revisions, split/fold/rebase, persistent typed conflicts and deterministic merges. Implement watcher-assisted auto-capture for admitted files, explicit protection status, coalescing, retention budgets, safe GC and explicit pinning.

Add a typed checkout-free library API with exact edit/file preconditions, versioned requests, structured conflicts, bounded streaming queries and durable idempotent outcomes. GUI/MCP/agent adaptors must use the same semantics, not shell scripts as the only truth.

**Gate:** 1/8/32 writers, stale generations, concurrent editors, watcher overflow, crash/GC interleavings, long-running readers, million-event bounded recovery traces. Explicitly exercise an in-flight prepared object racing collection, abandoned workspace lease, a rewritten dependency, and a nonoverlapping change with stale read assumptions. Several sessions using the same authenticated agent/person identity must remain distinct workspaces, and per-edit checks cannot require scanning all active workers' journals. Correctness and retention first, throughput second.

## Phase 3 — Git exchange and early interoperability

Import real Git source/history while retaining mappings to original objects. Export supported resolved native revisions as valid Git history. Integrate with one existing Git hosting path for proposals and reviews, honoring **host** authority for protected branches and CI. Do not silently share a Git index or worktree with native writers.

Maintain tested feature matrix: modes, symlinks, ignores, attributes/EOL/filters, tags/signatures, submodules, LFS, partial/shallow history and path encoding. Unsupported behavior must error before data loss, not be called compatible.

**Gate:** native/Git user collaboration, fixture round-trip for declared supported subset, no leakage of private recovery objects, explicit unknown-network-result reconciliation.

At this stage compare complete Git+worktrees, Jujutsu (including parallel isolated command execution), GitButler-style cooperative parallel branches where useful, and AVC workflows. Native development continues only if it demonstrates meaningful user value beyond a CLI variation. Do not claim original imported Git hashes persist after native revision rewriting, or that a Git adapter can enforce native transaction rules on an external host.

## Phase 4 — Native collaboration and validated landing

Build optional bounded native object sync and server: quarantine, closure/hash/authorization checks, restricted draft roots, proposal records, immutable combined candidates, exact test/approval evidence, target CAS, policy epochs, receipts and event outbox. No required forge, agent scheduler or CI runner. Existing host adapters retain their weaker atomicity semantics.

**Gate:** target advances/revocations during validation, nonoverlapping edits that break an API together, duplicate requests, cancellation, network partition and repeated event delivery. Include evidence for the wrong source digest, tampered test recipes, incomplete read sets, changed build environments and policy epoch changes. Only an exact candidate with evidence meeting the applicable policy can land. Distinguish a green test result from a guarantee of correct code.

## Phase 5 — Optimization based on evidence

Consider canonical paged wide directories, selective content-defined chunking, compressed segment packing, lazy/virtual working copies, scalable history/blame indexes, object-cache tiers, bounded merge drivers and asset publication locks. Require full-workload wins before adopting any technique.

## Falsification-oriented decision gates

The critical bet is that owning the VCS model enables an end-to-end advantage worth migrating from mature tools. Test that hypothesis on user workflows, not raw commit speed.

- **Data safety:** corruption, data loss, or incorrect acknowledged durability blocks progression regardless of speed. Power-failure claims require filesystem-specific evidence.
- **Native value:** separate workspaces, recoverable edits and operation history should measurably reduce failed handoffs, recovery loss, rework, or user effort versus tuned Git/Jujutsu. Agent-oriented features that merely duplicate jj or a merge queue fail this gate.
- **Local ordinary use:** opening, status, editing, diffing and restoring small/medium repositories must remain responsive without a daemon, custom filesystem or network.
- **Adoption:** a real Git project must import, perform a supported native workflow, export/submit to its current host and recover after interruption without undocumented behavior changes.
- **Scalability:** measured indexing/transfer/CI benefits must include cache misses, maintenance, GC, conversion, retries and shared-target validation. No extrapolation from Scion's internal fleet.

## Benchmark and assurance program

Compare maintained/tuned Git, current Jujutsu and native AVC with pinned source versions. Use ordinary projects and controlled large trees. Test clean/dirty status, edits to same/different files, 100k commits, up to 1m recovery snapshots, wide directories, binary churn, cold/warm history queries, 1/8/32 concurrent workers, sparse materialization, imports/exports, candidate integration and CI/event amplification.

Record latency distributions, CPU/RAM, bytes, object counts, GC, network, recovery time, stale/conflict rates, full workflow completion and failure behavior. Publish hardware, filesystem, command/config versions, seeds and raw results. Never apply Scion's author-reported fleet speedups to AVC as predicted gains.

Model-check core transitions; fuzz untrusted input; run crash/fault injection around every durability barrier, ref/metadata CAS, checkout replacement, GC and event acknowledgment. Qualify supported filesystem/OS behavior separately. Distinguish process termination from power loss. Independent backups and usable Git exports remain required during experimental development.

Current Rust checks remain:

    cargo test
    cargo fmt --check
    cargo clippy --all-targets --all-features -- -D warnings

Passing them does not establish the new contracts. Do not label native AVC safe for irreplaceable work until integrity, GC, restore and compatibility gates have been independently satisfied.
