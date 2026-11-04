# AVC — build and qualification plan

**November 3, 2026.** This is the **single active implementation plan** for the selected [architecture](design.md) and [workflows](ux.md). The November 2/early November 3 research-first roadmaps are superseded, not parallel action lists. The previous state is retained in [Git history](https://github.com/nijaru/avc/blob/94dd9921ee8a93b89551cbecf53bb14f16f08b01/docs/roadmap.md).

**Decision:** begin implementing the native vertical slice. Do not wait for a new wholesale design review. Do not freeze canonical bytes, migrate irreplaceable source, or claim safe custody until focused pre-freeze and crash-recovery gates pass.

## Verified current implementation boundary

On public `main` at the November 3 review, the crate has pure experimental BLAKE3 object encoding, `merkle::build` from an entire `FlatMap`, an in-memory object map, and `Verifier` that reconstructs directory maps and walks complete closures. The CLI only accepts informational flags; real operations refuse. It has **no durable repository engine, incremental persistent tree edit path, native Git bridge, checkout, recovery, remote, or usable CLI**. These pieces exercise an older, now-superseded byte format. Existing tests/research fixtures do not justify the claims of a native VCS.

This was verified by **source inspection**, not by running the crate: the connected GitHub source is available, but the current device checkout and Rust toolchain are not accessible from this chat session. Do not discard unrelated work in a local checkout when continuing.

## Milestone 1 — finalize narrow core bytes and perform actual persistent edits

**Implement** in the Rust library; avoid extending disconnected reference-only scaffolding.

1. Adopt the minimal [logical object model](design.md#1-canonical-logical-model-choose-snapshots-incremental-construction): exact blobs, typed deterministic Merkle directory nodes, first-class lossless source conflicts, authored revision roots/parents/ChangeIds. Remove obsolete experimental `State` wrapping and universal `Provenance` constructors unless a named invariant demonstrably needs them. No compatibility shims for unreleased prototype objects.
2. Implement persistent path-copy `get`, `put(expected)`, `remove(expected)`, subtree copy/move with explicit witnesses and limits, and Merkle `diff` that skips equal immutable subtrees. Do **not** reconstruct full directories for a one-path edit. Distinguish stored canonical objects from derived diff/changed-path hints.
3. Use deterministic path-compressed radix branching and compact sorted leaves provisionally. **One bounded native comparison** against deterministic chunked sorted pages must include: cold/warm path lookup, 1-path edit, batched edits, very wide flat directories, shared-prefix names, nested dirs, move, memory/allocation and encoded bytes. Previously published Python surrogates are useful priors, not native timings. Select precise canonical encoding/thresholds using these measurements and security checks. Document the winner and drop the losing implementation.
4. Validate exact typed, versioned BLAKE3 IDs with independent byte vectors, malformed/oversized records, duplicate names, kind confusion, symlink/path violations, and incremental validation from trusted verified roots. Full-tree verifier remains available **for audits**, not mandatory on every small edit.

**Gate:** order-independent canonical construction; correct O(touched paths and visited nodes) incremental edits on real structures; no missing source despite untrusted bytes; independent encoder/decoder validation. The exact format is still experimental before durable admission.

## Milestone 2 — single-machine durable source custody

**Physical implementation choice:** SQLite metadata authority plus immutable staged payload segments, with SQLite-only control in a narrow durability-equivalent experiment.

1. Add local object and root store with durable staged segment install, exact object verification and a **short root/pin/view CAS transaction** (WAL/FULL where supported). No unrelated directory writes. Root acknowledgement requires complete, durably installed owning closure; staged extras may remain unreachable after failure.
2. Add stable local reader/GC leases; pin ownership for active work, protected checkpoints, exports, and concurrent operations. Compaction must install replacement segments before metadata location updates and cannot retire a segment until all readers release it.
3. Add streaming file observation with complete disclosure of scope, explicit new-file/ignore policy, pinned candidate bytes and stale-result guards. `init`, `status` (read-only), `show`, `diff`, `commit`, `log`, and `verify` exercise one library path, not separate helper-only demonstrations.
4. Add stable self-contained native export/backup + independent restore. Backup pins an exact root and follows all owning dependencies; copying only the DB or only the segment directory is **not** a backup.
5. Add guarded source materialization with before-image protection and recovery of partially applied filesystem writes. Operation log records exact local view transactions and permits scoped metadata undo without silently altering external writer bytes.

**Gate:** inject failure after every staged write, sync, transaction, rename, ref CAS, compaction and cleanup boundary; restore acknowledged roots after process termination/power-loss simulation; demonstrate corrupt/truncated/missing object detection; refusal on stale filesystem writes. Demonstrate recovery with existing plain filesystem editors and concurrent read operations. Compare equivalent SQLite-only baseline for complexity, durability, read and ingest cost. **Switch to SQLite-only if hybrid cannot justify the extra failure surfaces.**

No user important-source custody until this is satisfied. Keep independent copies in Git.

## Milestone 3 — useful Git adoption end to end

Do this **before hosting or elaborate change-stack UI**. Implement an adapter using an established Git tool/library first; do not write a new Git protocol/packfile engine merely to demonstrate native semantics.

1. `avc clone git-url`: select exact Git default/explicit ref, retrieve **complete current tip source** for native conversion, hash into native roots and retain the exact Git foreign origin. Historical Git parents can be an explicit **unresolved foreign frontier**, never fabricated empty ancestry. User-visible history distinguishes verified native revision from foreign history not yet converted.
2. Edit/author native changes and export representable changes back to Git with stable foreign/native mappings, conservative ref compare-and-swap, and repeated fetch/push/rewritten-ref reconciliation. Verify the resulting files and ancestry with ordinary Git, not only an AVC reference model.
3. Perform a separate **filtered Git source** workflow on a large repo: use the adapter's read-only/conditional foreign view without claiming a native BLAKE3 root. Test that unsupported complete native authoring asks for hydration/translation rather than silently forcing a giant download. Record actual Git server filtering/partial-clone limitations and later-fetch costs.
4. Explicitly test submodules, LFS pointer vs payload, executable bits, symlinks, names/encoding, empty directories, merge parents, shallow Git boundaries, commit signing, unsupported typed conflicts and reimported rewriting. Only promise lossless conversion for a documented subset.

**Gate:** normal Git repo clone → native commit → Git remote push → independently verified Git content → upstream update → native merge/rebase or explicit incomplete-history error. No hidden dual writable authority over one checkout.

## Milestone 4 — history editing and conflicts

1. Implement correct merge bases for complete known ancestry; preserve unknown promised/foreign ancestry as **unknown**, not a root. Build deterministic merge from exact source inputs, versioned practical text driver and safe binary/tree/type/deletion cases.
2. Encode typed lossless source conflicts and test repeated rebase, compose, resolve, revert, conflicts of conflicts, staged file changes and resource-bounded refusal. **Narrow pre-freeze comparison:** simple typed alternatives versus normalized flattened expressions. Choose one with bounded complexity and no silently discarded terms. The old all-optimal-LCS algorithm is not the default unless controlled comparison clearly justifies it.
3. Implement stable ChangeId evolution across amend, rebase and scoped restack, and exact metadata operation-log undo; preserve old revisions under actual roots/pins, not forever through non-owning audit metadata.
4. Add selective path/hunk authoring with an immutable preview, no implicit staging-index model.

**Gate:** source and history semantics survive randomized bounded edit sequences, history rewrites, multiple merge bases, binary/path conflicts, and real representative repositories. No false-clean merge accepted merely for a high merge-success count.

## Milestone 5 — native partial clients and structured agent source APIs

1. Define authenticated root/ref resolution, typed object promise state, bounded/parallel batches, deferred fetch, idempotent resume and optional source query without working copy.
2. Separate history horizon, available current blobs/trees and materialized paths. Native clients can fetch selected current source and add files/history on demand. Full offline `hydrate --all` must verify complete requested custody.
3. Provide conditional source reads/writes against exact immutable source IDs; incomplete clients can ask an authorized full holder to validate a complete root. Avoid leaking unauthorized neighbor names via raw Merkle proofs.
4. Native remote publication uses full-closure durable availability followed by per-ref expected-version CAS and idempotent receipt. Read-only clients may populate local cache from remote, but source/history edits remain explicit.

**Gate:** large repository scoped clone → read a missing file → prepare a conditional update without checkout → concurrent competing agent updates → one clear success and one structured stale/ref conflict; retry safely and rehydrate. An offline partial client refuses any unknown dependency that prevents proving its result.

## Milestone 6 — composed changes and optional hosting scale

Implement local independent and dependent change stacks, explicit hunk ownership, pinned derived composition, optional `absorb` preview, and conflict-aware restack. Compare single shared developer/agent view against separate workspaces for setup cost, collisions, generated artifacts and service state; do not require a daemon or full checkout merely for source composition.

For hosted scale, apply proven principles from Pando/Scion/Mononoke: immutable tiered caches, derived indexes, reusable transport groups, ref-local concurrency, and optional server-side query/transform. Keep client and server source semantics shared. A hosted implementation is **not** required for a trustworthy local VCS.

## Performance evaluation

Measure **real entry-point tasks**, not just hash throughput, and compare to Git and Jujutsu/Sapling where tasks are semantically equivalent. Use at least a small active code repository, a large monorepo with wide/deep directories, a binary-heavy project, incremental frequent agent edits, and cold/warm repeat workloads. Record environment, exact input fingerprint, memory, CPU, disk I/O, storage amplification, wire bytes, p50/p95 latency, remote round trips, and failure/retry behavior.

Do not extrapolate Scion's 100x or Pando's 5x claims to AVC. BLAKE3 and Zstandard must be measured **inside** complete clone, edit, diff, ingest and hydration tasks; compare against Git's delta-packed storage. Prune misleading or no-longer-applicable old fixtures, but preserve valid adversarial safety and contract coverage.

## Explicit format-freeze requirements

Before admitting irreplaceable native data or declaring a stable format:

- Canonical object encodings and selected radix map are independently validated against malformed/adversarial input and incremental updates; IDs have versioning and migration policy.
- First-class conflict representation preserves source alternatives under repeated history editing and bounded failure; no ambiguous clean merge.
- Local durable ack, staged segment loss, GC, checkpoint retention, foreign refs, export and restore survive fault injection on supported platforms.
- Full versus partial clone promises are unambiguous; native current root cannot be fabricated from unfetched Git content.
- Standard Git adoption completes the practical round trip for an explicitly supported subset.
- Real CLI and library checks pass; source-code behavior agrees with these documents.

The architecture is **selected enough to build**. These are verification gates, not invitations to reopen every architectural choice at every release.

## Next local/device session: immediate actions

1. Inspect `git status`, branch/remotes and latest commits under `~/github/nijaru/avc`; preserve unrelated work and any preexisting `.avc` state. Load `nijaru/dotfiles/instructions/AGENTS.md`, this roadmap, `design.md`, `ux.md`, and `nijaru/knowledge/projects/avc/constraints.md`.
2. Migrate the **existing codec and tests coherently** to the smaller logical object model (no compatibility commitment to old draft), replacing whole-FlatMap merkle construction with persistent conditional path-copy updates. Keep one authority per concern.
3. Run one native radix-versus-chunked map comparison before freezing actual node bytes/thresholds; document results and discard the rejected variant.
4. Build object admission and root transactions into a real `init` / `commit` / `show` / `verify` vertical slice. Run crash and corruption tests before calling it custodial.
5. Implement ordinary Git tip import and outbound exchange after the local core works; keep large partial-Git foreign view separate, with no misleading native-root claims.
6. Reevaluate after measured tasks; do not produce another research-only architecture round without a concrete violated invariant or demonstrated end-to-end cost.

**No new application, project manager, handoff file or separate private specification is required.** Update this plan and the single owning docs as observations actually change them.
