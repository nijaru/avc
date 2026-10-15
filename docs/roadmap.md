# AVC — design qualification and delivery roadmap

**Revised 2026-10-15.** [Design](design.md) is the selected v0 architecture; [research](research.md) records evidence and rejected choices. The format is *not yet published/stable* and the current Rust 0.0.1 code remains an unrelated Git-command prototype.

**This is no longer a roadmap to decide which VCS AVC should be.** It is the plan to validate and realize one specified design. Architecture changes require explicit correctness findings or substantial full-workflow evidence, not a comparison of fashionable upstream projects. No implementation code was changed by the design decision.

## Milestone 0 — complete the binary and failure contracts

Before permanently writing native user data, specify and review:

1. **Canonical typed object encoding:** BLAKE3-256 domain separation, lengths, versioning, bounded raw path components, file kinds, small/large radix map nodes, sorted leaf entries, prefix compression, deterministic normalization, empty directories, revision and ChangeId hashing. Publish golden vectors from an independent reference decoder. This is *parameter/codec qualification*, not a new history-model vote.
2. **Pure source model:** reference operations for Put/Remove/CopySubtree/MoveSubtree with exact expected old value and verifiable absence. Same logical tree from different edit order yields identical StateId; tree copy/move does not rewrite descendants. Test adversarial keys and huge directories.
3. **Conflict source algebra:** exact three-way value rules, typed absence, file-vs-directory, symlink/binary, multiple merge-base recursion, already-conflicted operands, preservation of nested alternatives, bounded expression DAG, canonical normalization and separate ConflictOrigin. Define marker-view round-trip without turning unchanged markers into source bytes.
4. **Revision evolution:** divergent amendments, single-parent rebase, split/fold explicit lineage, logical undo/revert/reapply as different operations, title edits and descendant identity costs. Reject undefined multi-parent rebase rather than silently reinterpreting it.
5. **Workspace and capture state machine:** separate selection/capture/materialization generations; acknowledgement and retention pins; per-file observation versus quiesced snapshot labels; stale watcher/capture/selection races. Fresh-directory restore safety, materialization-progress journal and actual filesystem writer limitations.
6. **Object durability and GC:** SQLite root/outcome authority, BLAKE3 closure, staged external blob writes before root CAS, directory fsync assumptions, in-flight and reader pins, GC and backup consistency, lost reply/replay/epoch idempotency.
7. **Native exchange security:** full/partial promises, authenticated sparse tree/path reads and bounded bundles; secure object-closure authorization; durable root acknowledgment before an ephemeral VM exits.
8. **Shared acceptance:** exact candidate identity, versioned validation policy and evidence, target-specific CAS, cancellation/retry/outbox, optional accepted-versus-verified policy.

**Exit gate:** specification is internally non-contradictory and covers the significant failure/format cases. No build or storage optimization should override it. Use independent review/model-checking where correctness of the composition semantics is subtle.

Existing [in-memory source-model oracle](experiments/source_models_oracle.py) has 14 semantic tests; [contract probes](experiments/vcs_contract_probes.py) demonstrate narrow earlier counterexamples. Both remain **research fixtures**, not native engine benchmarks, format proofs, or storage durability evidence.

## Milestone 1 — native local source custody

Replace the legacy Git wrapper through one cohesive native library/CLI; retain old repositories without silent reinterpretation. Build the typed content objects and deterministic radix source tree, StateId, RevisionId, logical ChangeId/evolution, local SQLite metadata authority and small-object store.

A complete local user should be able to initialize, inspect status, record a revision, list/history/diff/show exact source by ID, move/copy a subtree, reopen the repository, create a separate workspace, and restore safely to a newly allocated directory **without Git, network or daemon**.

**Hard gates:** every acknowledged root has complete durable object closure; wrong hashes reject; no path escape or silent name collisions; equal source content produces equal canonical StateId; incomplete filesystem observations are labeled honestly; recover after injected process crashes and disk-full conditions. Native verified export/backup supports independent source custody. Run actual Rust tests/lint/toolchain checks once Rust code changes.

## Milestone 2 — recoverable work and history composition

Implement distinct capture/head/selection generations and bounded retention. Work through typed conflicts, symbolic markers with round-trip manifest, conservative content merging, rebase/amend evolution, source revert, operation undo and explicit reapply.

Cases: competing agents (choose one), compositional independent agents (combine selected changes), same-path edits, directory move/edit, delete-vs-modify, binary conflicts, already conflicted source, criss-cross multiple bases, concurrent workspace status/capture, crash between root selection and directory creation.

**Hard gates:** no loss of conflict alternatives; no permanent clean claim about uncontrolled files; no acknowledged capture missing its bytes/pin within retention; no in-place overwrite of unseen edits under a "safe" guarantee; conflicting proposals remain recoverable; no spurious shared logical head contention. Tests run against **immutable built source**, not mutable worker files.

## Milestone 3 — native source protocol and ephemeral handoff

Expose the same typed semantics in a library and native protocol: get file/range/tree by source ID, diff, history/ancestry, conditional source transform, merge, draft sync and protected target update. Implement resumable batched transfer plus sparse object hydration; full copied repositories must operate offline.

Critical end-to-end test: full client chooses a revision; an isolated container/microVM receives the needed immutable source (without a full history clone), edits ordinary files, submits a **durably pinned immutable authored revision** and terminates; a second machine reconstructs exactly the submitted source. Repeat under network failure, retries, missing partial objects, restart, authorization revocation and GC.

**Hard gates:** an unpinned upload is never reported as a durable handoff; remote source reads honor rights to the allowed rooted closure; no information leaked simply by guessing object hashes; no client interprets promised-but-absent content as missing source.

## Milestone 4 — shared integration and candidate validation

Implement candidate/recipe identities, validation attestations, shared target policy and compare-and-swap, batching selected work, accepted/verified options and durable publication receipts. Integrate native source-state equality with external test/build provenance without incorporating CI into the VCS engine.

**Hard gates:** tests apply to the exact accepted candidate and declared environment; stale target/policy/auth inputs invalidate relevant evidence; concurrent proposals remain preserved; one linear target has honest serialized acceptance semantics; retries after unknown commit responses are idempotent.

## Milestone 5 — performance and platform qualification

**Do not reopen the architecture merely because a microbenchmark favors another encoding.** Tune the selected bounded radix layout, physical file/blob thresholds, compression/chunking, derived ancestry/path/blame indexes, cached sparse/bulk transfer and hosted scale with representative workloads.

| Benchmark | End-to-end measures |
| --- | --- |
| Human edit-test-repeat, tiny and medium repos | status, revision write, diff, first read, memory and source recovery |
| Private agent saves, millions of bounded captures | durable admission rate, quota behavior, retention reclamation |
| 1/8/32 independent writers | throughput, time spent in physical write transactions, false logical conflicts |
| 100k revisions, large trees and wide directories | cold/hot path lookup, diff, subtree moves, canonical node counts, GC |
| CI and remote microVMs | cold source acquisition, sparse API calls, transfer bytes, shared-cache effects |
| Real merges and refactoring histories | wrong auto-merge rate, unnecessary conflict rate, latency and review burden |
| Process kill/power loss/disk full/remote outage | acknowledged source survival, staging/GC races, backup restore, recovery time |

Use comparable durability, source content and wall-clock total workflow against tuned Git/Jujutsu and relevant existing alternatives. Report software versions, machine/filesystem, raw measurements, cache warmth, retries and failures; do not present Scion/Enroute's fleet figures as AVC speedups.

**Platform qualification:** Linux filesystems, APFS and Windows have different rename/fsync/symlink/name rules. Test the supported feature subset explicitly. Treat a successful process-kill experiment as *insufficient* proof of power-loss safety.

## Deferred independent questions

Only engineering parameters and discretionary integrations remain: precise leaf thresholds, blob/chunk compression choices, portable source metadata expansion, future hosted physical cache layout, and any optional Git/import/export bridge. These do **not** prevent explaining or implementing the selected architecture. Keep the project named AVC; do not add a required Git-compatible mode, hosted forge, process sandbox, or custom consensus layer.

**Release rule:** trust source integrity, explicit undo/merge behavior, capture/restore honesty and independent export/backup before encouraging users to entrust irreplaceable code. No native implementation or verified production speedup is implied by this design package.
