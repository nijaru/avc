# AVC — design qualification and delivery roadmap

**Revised 2026-10-15.** [Design](design.md) is the selected v0 architecture; [research](research.md) records evidence and rejected choices. The format is *not yet published/stable* and the current Rust 0.0.1 code remains an unrelated Git-command prototype.

**This is no longer a roadmap to decide which VCS AVC should be.** It is a qualification plan for the selected but not yet independently validated contracts. Architecture changes require explicit correctness findings or substantial full-workflow evidence, not a comparison of fashionable upstream projects. No implementation code was changed by the design decision.

## Milestone 0 — independently verify the selected contracts

The [authoritative design](design.md) now selects the source authority, typed object families, BLAKE3 domain-separated framing, directory map algorithm, conflict grouping, workspace/recovery contracts and commit/GC ordering. **Do not invent a new architecture in implementation.** Before permanently writing native user data, verify the specified rules with **independent** decoders/models and golden vectors:

1. **Canonical typed object encoding:** independently reproduce BLAKE3 framing, **typed external ID**, kinds 1–9, exact 4096-byte leaf-size inequality, nibble prefix compression and parent-context parity, fixed-width lengths, path bounds and canonical map reconstruction. Publish and cross-check byte+hash vectors for empty map/source, files, symlinks, prefix-name pairs, threshold branches, odd-nibble boundary, directory moves, root conflicts, grouped conflicts, Revision/Provenance and PublicationCandidate; reject hash mismatches, missing closure, bad padding, invalid root types and noncanonical alternate trees. Two independent decoders/builders must agree before the first **durable native repository** is created.
2. **Pure source model:** independently check Put/Remove/CopySubtree/MoveSubtree as a conditional two-phase structural+leaf transaction, including exact old/absent witnesses, overlapping/swap rejection and retention of referenced copy sources. Random edit order yielding the same logical tree must produce the same StateId. Test huge directories, ancestor/descendant keys and all 4095/4096/4097 leaf boundaries.
3. **Conflict source algebra:** independently exercise equality/base rules, **unique-optimum LCS** line anchoring under resource limits, CRLF/duplicate-lines and text/binary/mode/type cases; verified user-recorded moves versus heuristic lookalikes; deletion/rename coupled subtree/root conflicts. Confirm **width >64** canonical kind-8 grouping and that **nesting >64/graph-budget overflow refuses** a new State with protected input roots. Model criss-cross frontier reduction with virtual ancestry, already-conflicted inputs and absent/non-directory descendants; read unaffected siblings without wrongly assuming a missing sparse object is absence. Marker round-trip cannot resolve conflicts implicitly.
4. **Revision evolution:** divergent amendments, single-parent rebase, split/fold explicit lineage, and generational operation undo versus exact **inverse-content revert/reapply** formulas, including multi-parent mainline selection. Verify separate Revision/State/ChangeIds and revision/provenance/evolution acyclicity; reject unrequested descendant rewrites and undefined merge-commit rebases.
5. **Workspace and capture state machine:** separate selection/capture/materialization generations, acknowledgement and retention pins, observation coverage, watcher gaps and honest writer-quiesced labels. Test unknown untracked obstacles, symlink containment, dirty/clean races, journal writing before the first mutation, guarded inode/path replay, verification timestamps and `Incomplete` recovery after process kill. Distinguish strong **fresh-directory** no-clobber restore from inherently racy **in-place** switches.
6. **Object durability and GC:** prove admission ordering against actual SQLite WAL/FULL + external files. Model and inject **mark-then-concurrent-pin-then-sweep**: reclaim candidates require transactionally checked `Deleting` tombstones, and publishers/readers reject tombstoned dependencies. Test file/dir fsync, GC/object install/path-reuse races, orphan staging, idempotency key replay after unknown replies, online-backup-versus-GC leases and reconstructable export/restore.
7. **Native exchange security:** distinguish proven absence from promised/missing objects; validate complete-root sparse witnesses, **do not disclose raw siblings to path-restricted clients**, and verify server-side scoped transforms under authorization changes. Exercise bounded resumable bundles, cross-tenant guessed-ID queries, and durable pinned draft **receipt+closure** before microVM termination.
8. **Shared acceptance:** independently reproduce kind-9 PublicationCandidate byte identity and validation evidence bound to it; check policy/toolchain/source freshness, protected ancestry and authorization. Model monotonic target-generation CAS (including ABA), unknown-result retries with same/different payloads, durable receipts and at-least-once outbox deduplication. Specify signed evidence format and validator trust policy **before** the publication protocol ships.

**Milestone-0 exit gate (currently NOT met):** at least two independent object codecs agree on published golden vectors; a separate pure reference model exercises the corrected structural/merge/recovery invariants; and modelled GC/pin, workspace and publication interleavings have no unresolved loss/cycle/ABA counterexample. Record failed cases explicitly. A needed change to an unreleased constant must be justified and versioned **before any persistent native data** is entrusted to it. Model checking of the concurrent state machines is a release qualification, not proof from reading this document.

Existing [in-memory source-model oracle](experiments/source_models_oracle.py) has 14 semantic tests; [contract probes](experiments/vcs_contract_probes.py) demonstrate narrow earlier counterexamples. Both remain **research fixtures**, not native engine benchmarks, format proofs, or storage durability evidence.

## Milestone 1 — native local source custody

Replace the legacy Git wrapper through one cohesive native library/CLI; retain old repositories without silent reinterpretation. Build the typed content objects and deterministic radix source tree, StateId, RevisionId, logical ChangeId/evolution, local SQLite metadata authority and small-object store.

A complete local user should be able to initialize, inspect status, record a revision, list/history/diff/show exact source by ID, move/copy a subtree, reopen the repository, create a separate workspace, **switch in-place with protected before-images and honest limitations**, and restore safely to a newly allocated directory **without Git, network or daemon**.

**Hard gates:** every acknowledged root has complete durable object closure; wrong hashes reject; no path escape or silent name collisions; equal source content produces equal canonical StateId; incomplete filesystem observations are labeled honestly; recover after injected process crashes and disk-full conditions. Native verified export/backup supports independent source custody. Run actual Rust tests/lint/toolchain checks once Rust code changes.

## Milestone 2 — recoverable work and history composition

Implement distinct capture/head/selection generations and bounded retention. Work through typed conflicts, symbolic markers with round-trip manifest, conservative content merging, rebase/amend evolution, source revert, operation undo and explicit reapply.

Cases: competing agents (choose one), compositional independent agents (combine selected changes), same-path edits, directory move/edit, delete-vs-modify, binary conflicts, already conflicted source, criss-cross multiple bases, concurrent workspace status/capture, crash between root selection and directory creation.

**Hard gates:** width grouping never loses a term; an over-budget merge fails **without** an invented output State but keeps explicitly pinned inputs. No permanently clean uncontrolled files; no acknowledged capture missing bytes within retention. Default in-place switching refuses dirty/untracked/unknown obstructions, preserves before-images and does not overwrite post-crash external edits. Conflicting proposals remain recoverable; separate workspaces incur no shared logical head contention. Tests run against **immutable built source**, not mutable worker files.

## Milestone 3 — native source protocol and ephemeral handoff

Expose the same typed semantics in a library and native protocol: get file/range/tree by source ID, diff, history/ancestry, conditional source transform, merge, draft sync and protected target update. Implement resumable batched transfer plus sparse object hydration; full copied repositories must operate offline.

Critical end-to-end test: full client chooses a revision; an isolated container/microVM receives the needed immutable source (without a full history clone), edits ordinary files, submits a **durably pinned immutable authored revision** and terminates; a second machine reconstructs exactly the submitted source. Repeat under network failure, retries, missing partial objects, restart, authorization revocation and GC.

**Hard gates:** an unpinned upload is never reported as durable handoff; source read permission is checked on rooted closure and path scope; no raw sibling proof disclosure beyond policy; guessed hashes do not bypass authorization; no client interprets promised-but-absent as source absence; unknown submit responses are resolved through durable idempotency receipts.

## Milestone 4 — shared integration and candidate validation

Implement candidate/recipe identities, validation attestations, shared target policy and compare-and-swap, batching selected work, accepted/verified options and durable publication receipts. Integrate native source-state equality with external test/build provenance without incorporating CI into the VCS engine.

**Hard gates:** signed tests apply to exact candidate, declared environment/toolchain and trusted policy; stale target/policy/auth inputs invalidate acceptance; protected ancestry rules are checked; concurrent proposals remain preserved. A linear target serializes **only its own** acceptance, with monotonically increasing generation and persisted receipt/outbox. Retries after unknown responses are idempotent and duplicate side effects are rejected.

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

Engineering qualification and discretionary integrations remain: published **independent codec vectors and parser limits are mandatory Milestone-0 gates**, not optional physical optimizations. Physical blob/chunk compression, future source-metadata expansion, hosted cache layout and a potential separate Git/import/export bridge are discretionary later choices. The **4096-byte canonical map leaf rule and nibble partition** are already selected v0 codec rules, not unspecified knobs. These do **not** prevent explaining or implementing the selected architecture. Keep the project named AVC; do not add a required Git-compatible mode, hosted forge, process sandbox, or custom consensus layer.

**Release rule:** trust source integrity, explicit undo/merge behavior, capture/restore honesty and independent export/backup before encouraging users to entrust irreplaceable code. No native implementation or verified production speedup is implied by this design package.
