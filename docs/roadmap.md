# AVC implementation plan and release gates

**Updated 2026-11-09.** The architecture has been **selected and implementation has begun**. This is the single actionable plan for local development. There is no parallel research-first redesign track. Architecture [design](design.md); canonical bytes/conflicts [format](format.md); local custody [storage](storage.md); native sync [sync](sync.md); Git adapter [git](git.md); CLI [UX](ux.md); rationale [research](research.md).

**Present state:** a new Rust library and experimental source-only repository/CLI now implement the current format in part. Independent Python vectors and current-format tests replace the retired prototype; this is not a migration of published data. The obsolete codec, old tests and studies remain intact in [the pinned previous repository revision](https://github.com/nijaru/avc/tree/af66a63898f383b596e4828199dc625a80e28a72), not as current normative code. Their old State/provenance kinds, 4 KiB leaves, driver and exact IDs remain superseded. No qualified source custody, authored-history CLI, workspace capture, Git adapter or sync service exists.

### Implemented slices and next gate

- **Stage 0:** new library/CLI, CI configured for macOS/Linux, independent v1 Python encoder/decoder and fresh canonical/malformed corpus. Current reference: 35 objects, 23 entry points, 50 negative corpora; Rust/Python cross-check generated maps in both directions.
- **Stage 1:** conditional path-copy radix edits, simultaneous witnessed move/copy/swap, cached verified bounds, source reads, threshold-safe Merkle diff and separate full contextual verification. Random edit orders/rebuild oracles and wide/shared-prefix cost probes exercise structural sharing. Heap-stack traversals replace the source/radix recursion that overflowed a 2 MiB debug stack. Bounded adversarial deep-radix and full-path fixtures now pass on that stack in debug and release; canonical path bounds were not lowered.
- **Stage 1A:** focused two-agent composition and source witness cases, signed rebase/revert counterexamples, conservative witnessed relocation composition, and a real local blobless Git-relative tree edit. Review found and corrected divergent-resolution false-clean cancellation, root-Absent legality and conflicted-content mode loss; [research §13](research.md#13-first-native-qualification-and-demonstrated-corrections-november-5-2026) records failed-before-fix evidence and limits. These are pure source tests, not a complete authored-history implementation.
- **Stage 2, initial vertical slice:** experimental `init/status/show/diff/verify/export/import/restore/receipt` and checkout-free `source` commands on one SQLite store (physical schema 2: payloads, selection and receipts share each transaction), with generation CAS, protected before/after source receipts, reopen, independently checked export/import, no-clobber fresh-directory materialization, concurrent writers, corrupt-object refusal and process termination around commit. Installation-sync and identical-request preparation races have failed-before-fix regressions. Explicit size limits and unimplemented physical/metadata features are in [storage](storage.md#current-implementation-boundary-november-2026). Reads do not scan/checkpoint the filesystem; source updates do not author revisions.

**Next qualification:** complete the Stage 2 fault/restore matrix on the schema-2 store. The deterministic native fault suite (child `d05db92`) completed on macOS against the previous schema-1 hybrid — 210 source, 117 receipt, 51 export cases; Linux runs and final qualification against schema 2 are pending and not claimed. Then scoped observing authority, authored refs/revisions and retained operation/workspace recovery. Complete persisted incremental verification, disk-full/sync faults and OS power-loss qualification before claiming custody. The storage-control comparison is complete ([research §14](research.md#14-sqlite-only-storage-decision-and-equivalent-work-control-november-6-2026)); SQLite-only schema 2 is the permanent bounded-model backend. Stages 3–7 remain unimplemented; no format freeze or claim that these earlier slices satisfy all release gates.

Current canonical checks are in the README. The independent reference's `--vectors` checks both byte grammar and rooted closure; `AVC_REFERENCE_CHECK=1 cargo test --locked --offline` also checks newly generated Rust roots against Python. The pure cost probe is `cargo run --release --locked --example source_cost -- 10000 [shared]`, not an end-to-end VCS benchmark.

## Stage 0 — inspect local checkout and create the new crate

1. Work in `~/github/nijaru/avc` on the actual machine; read the global `nijaru/dotfiles/instructions/AGENTS.md`, inspect `git status`/branch/remotes, verify docs and preserve local uncommitted work. Never discard existing `.avc`/Git data; unsupported legacy `.avc` is refused.
2. Start one Rust workspace with a reusable core library and thin `avc` CLI. Organize by canonical objects/source trees, repository storage, working directory, sync, Git bridge and presentation **only as needed by implemented operations**. Avoid speculative provider traits for a hosted product that does not exist yet.
3. Restore focused Rust formatting/clippy/build/test CI *for the new implementation*; do not reinstate old codec checks. Use versions governed by shared project instructions; no invented release milestone or autonomous PR.
4. Construct an **independent format reference checker** (small separate language implementation or strict reference vectors) for v1 canonical ID, map, conflict and revision payloads. Independently pin vectors for empty map, regular file, symlink, wide map split/collapse, signed conflict, authored revision and malformed inputs. Check the normative grammar in [format.md](format.md) before saving first durable source.

**Gate:** two independent decoders agree on exact bytes/IDs and reject noncanonical input. Correct the unreleased format document and all fixtures together if a genuine defect is discovered.

## Stage 1 — real persistent source engine

Build immutable content objects and path-copy tree edits, not in-memory `FlatMap` reconstruction. Required Rust library operations: typed hash/encode/decode, `map.get`, `map.put(expected)`, `map.remove(expected)`, `map.copy/move(witness)`, `source.read`, and Merkle diff that skips equal subtrees. Include exact path/symlink validation, case/OS representation failure, and resource-bounded full verification independent of incremental trusted-subtree verification.

Use the **selected radix/32-leaf format**. No whole-system A/B architecture vote remains. Benchmark wide and shared-prefix directory edits, random insertion orders, empty directory retention, moves with/without trusted cached bound summary, cold lookups, Rust allocation, and incremental edit bytes. If real results expose an unacceptable failure, **change the unreleased format coherently**, not preserve a flawed threshold to match this plan.

**Gate:** exact source equivalence to an independent flat-path oracle across randomized operation sequences; order-independent canonical roots; typed invalid input rejection; no need for full-tree walks on a normal one-file edit.

## Stage 1A: Research resolved

**Research disposition is already recorded in [§12](research.md#12-evidence-first-resolution).** Git `merge-tree` and GitHub's Trees API already implement checkout-free source operations; Pijul records causal changes from ordinary edited working directories; Jujutsu documents signed conflicts and the same-change intent-loss counterexample. Published merge evaluations already contain **6,045 real-world merge scenarios**, and a 2026 study replays **747 agent-authored PR pairs**. **Do not reproduce these broad demonstrations as prerequisite feasibility studies.**

After the Stage 1 pure engine works, run only three **bounded** AVC checks:

1. **Exact source composition contract:** two agents propose conditional edits against one immutable source root; compare disjoint edits, conflicting edits, stale witnesses, move/delete and incomplete source proofs. Return verified roots or typed refusal. This is conformance, **not a claim of novelty over Git**. Preserve known typed operation witnesses where useful; do not invent a second canonical patch store.
2. **Conflict/change semantic counterexamples:** port a few cases from Jujutsu's documented same-change loss, Pijul's causal order/dependency examples and AVC's historical tests; include repeated rebase/revert, conflict resolution then rebase, binary/type/rename/delete. The signed-conflict v1 format is **not frozen** until these pass or the spec is corrected. Use the established [ASE 2024 evaluation corpus](https://github.com/benedikt-schesch/AST-Merging-Evaluation) for later broader merge-driver testing; no need to implement Pijul itself.
3. **Git boundary proof:** for a representative filtered Git tree, prove the foreign blob OIDs cannot be treated as native BLAKE3 IDs, validate a Git-relative edit using normal Git APIs, and specify the optional **shared server-side conversion/cache** alternative. Measure actual tip conversion and partial-client tradeoffs during Stage 4's real Git adapter, not as a speculative custom clone benchmark now.

**Gate:** the new engine passes these contract tests; no demonstrably unsafe conflict semantics or false native-root claim. Published research already supports the architecture direction; **do not make a large competitor benchmark a prerequisite for creating the durable local store**. Compare complete agent tasks against Git+jj/GitButler once AVC actually supports equivalent tasks. A concrete failing counterexample can still overturn unreleased source/conflict choices.

## Stage 2 — durable local repository and safe source custody

Implement [storage.md](storage.md): the SQLite WAL/FULL store (physical schema 2) owning payload rows, selection and receipts in one transaction, bounded closures, idempotent receipts, object verify/export/restore. Compression, segments/extents and streaming beyond the bounds are deferred to measured need. Add the actual CLI `init`, `status` (read-only), `show`, `diff`, `commit`, `log`, `verify`, and `export` on **one** source engine. Use no Git backend, daemon or mandatory service.

Implement exact scoped filesystem capture with pinned selected bytes; preflight path/symlink and stale-write guards; guard non-atomic filesystem materialization with before-image protection and recoverable operation receipts. Default recovery into a new directory. Add explicit checkpoint with truthful scope/retention and private work handles before history convenience.

**Mandatory fault matrix:** fail at each SQLite begin/commit, quota/disk-full condition, sync call, ref CAS, backup and restore stage, and materialization rename/removal. For every acknowledged root verify byte-perfect restore after crash. Test corrupt/unexpectedly missing object rows, denied mutation boundaries, concurrent reader/writer, and case/symlink escape. The equivalent-work control comparison decided the physical backend ([research §14](research.md#14-sqlite-only-storage-decision-and-equivalent-work-control-november-6-2026)); there is exactly one store to qualify, and no second physical backend is maintained.

**Gate:** full native offline recovery independent of any server. Until this passes, operational commands must be labeled experimental or refuse important-source custody; the repository has no claim to protect irreplaceable work.

## Stage 3 — authoring, history and first-class conflicts

Use the Stage 1A change/merge counterexamples as input. Implement immutable Revision DAG, random stable ChangeId, selection/ref CAS, amend/rebase/restack, exact lineage and scoped local operation-log undo. Implement recursive known-base merge and the selected normalized signed conflict objects, with lossless alternate binary/path/type/source values, bounded deterministic text driver, resolution, revert and repeated conflict rebase.

Test incomplete/promised ancestry, **typed `AmbiguousMergeBase` refusal and explicit verified base selection** on multiple maximal bases, simultaneous conditional subtree moves and path-witness collisions, partial path edits, overlapping same-file hunks, symlink and directory transitions, merge of conflict-valued source, `absorb` suggestions on clearly attributable versus ambiguous hunks, and resource-failure-as-pending (not fabricated conflict).

**Gate:** repeated history rewrite and merge cannot silently discard source alternatives or invent parents; exact revision selection remains recoverable across crash and GC.

## Stage 4 — Git onboarding early

Build on established Git partial-clone documentation and the bounded Stage 1A source-identity check. Implement [git.md](git.md) against mature Git client/library code, not a new Git pack protocol: exact selected-tip source conversion to native, **deterministic import anchor identical across two clients given the same Git commit/source**, foreign ancestry frontier, remote tracking, safe Git commit/tree export, push with lease and lost-response reconciliation. Validate with *actual Git* on a real small repository, not just AVC oracles.

Provide an **explicit** foreign partial view for massive filtered Git repos, including Git-relative path queries and conditional Git-native commits/push that reuse unchanged Git object IDs without full native import. Do not claim source-specific BLAKE3 IDs for unfetched blobs. Full native import from foreign view is a separately invoked hydrate/convert action. **Later hosting option:** if repeated clients suffer from full-tip native conversion, evaluate a shared, cacheable Git→AVC conversion service that reads/verifies all necessary Git bytes once and supplies authenticated native roots/objects to thin clients. This does not eliminate conversion work or create a second native history authority.

**Gate:** Git clone→native edit→native commit→Git push→independent Git byte/parent verification→upstream change→safe fetch/rebase/push; a blobless foreign agent can do Git-relative work without a false native root.

## Stage 5 — native partial-clone and checkout-free agent API

Implement [sync.md](sync.md): exact ref resolution, explicit object availability/promisor states, authenticated Merkle path proofs, path-scoped conditional root edits, grouped/resumable native transport, offline/full hydration, and remote ref CAS with durable object closure and idempotent retries. The remote service uses the same canonical library operations; do not add a second server-side source authority.

**Gate:** large native repo scoped/no-checkout read→edit→candidate root using sibling proof→server completes closure→two agents race to publish→one succeeds/other gets `StaleRef`→safe rebase/retry; offline full hydrate/export works while partial clone reports missing data precisely.

## Stage 6 — composed changes and hosted scale

Extend the Stage 1A **minimal derived composition API** into optional GitButler-style independent/dependent local stacks and reproducible composed source views, explicit hunk assignment, safe `absorb` preview and candidate review/test references. Verify isolation modes: distinct directories, shared directory under cooperative use, and checkout-free conditional agents; measure setup versus collisions and hidden dependency effects.

Only then profile hosting throughput and implement derived path history, blame, rename hints, cache tiers, reusable transfer packs, Zstandard dictionaries and physical compaction policy where justified. None of these changes source IDs. Hosted features, Canto integration, CIs and forges are separate products/clients.

## Stage 7 — optional Git protocol frontend on native hosting

**Only after** native local custody, basic Git remote bridge and native hosted CAS are reliable. This is a *compatibility interface*, **not** another Git-backed AVC storage engine.

1. Implement deterministic derived Git blob/tree/commit projection and a versioned mapping/cache for Git-visible **representable** native revision closures; refuse conflict/empty-directory/unrepresentable/incomplete histories unless the user explicitly selects a separate projection.
2. Implement Git protocol v2 fetch and ref advertisement backed by the **same authoritative native ref generation**, not an independently advancing shadow Git ref. Validate with stock Git clients and standard pack-object/reachable-object rules.
3. Implement Git receive/push translation: validate received Git OIDs/pack/commit ancestry and permissions, convert complete supported inputs to native revisions, enforce native CAS plus Git old-OID/fast-forward or explicit force-lease conditions, then durably acknowledge. Test races between Git and native publishers and response loss after commit.
4. Add shallow/deepen, partial-clone filtering, tags and physical cache improvements only when compatibility demand justifies maintaining each capability.

**Gate:** native and Git readers see the same representable shared ref; concurrent Git/native pushes cannot lose a committed update; native-only states never masquerade as clean Git history. A Git client can fetch/push supported source without AVC storage becoming Git-backed. Unsupported source or missing history produces clear refusal and recovery guidance.

## Release standards and evidence

Before public stable bytes or important-source custody, require:

- Independent codec vectors plus malformed inputs, fuzz and property oracles, including signed expression normalization, branch split/collapse and revision ancestry.
- Durable complete-root acknowledgement, GC safety, exact backup/restore and interrupted file switching on supported macOS/Linux filesystems; no source loss under declared failure model.
- Explicit partial/foreign/missing/unauthorized states; authenticated membership/absence and no sibling-name leakage for restricted clients.
- **Comparative product gate:** compare AVC against modern Git+Jujutsu, GitButler/Graphite stacks, and relevant Pijul/DeltaDB agent-edit models where task semantics overlap. Trial at least (a) normal Git clone/edit/push, (b) huge current-source path-scoped agents, (c) multiple concurrent source edits and restacks, and (d) interruption/recovery. Capture **time to first useful edit, complete-task p50/p95, network bytes, RAM/SSD, manual conflict fixes, false-clean results and safe recovery** under matched conditions. Judge foreign partial Git mode separately from native AVC. See [competitive falsification record](research.md#10-november-4-competitor-steelman-after-the-one-native-backend-decision). An incremental Merkle microbenchmark alone cannot justify an independent VCS.
- Representative human and agent tasks against Git/Jujutsu where semantics match, cold/warm cache, SSD/RAM/CPU/network bytes, p50/p95, later history deepening and change-conflict burden.
- Git interoperability with documented supported subset and exact refusal of lossy states; no silent move of foreign or public refs.
- No implicit execution of untrusted source hooks or unsafe symlink/path traversal.

**The core architecture is selected; the early Stage 1A experiment is a targeted product/semantic falsification gate, not a second architecture project.** There are no unresolved architecture choices requiring user input. Any future design change must cite a concrete failed invariant, uncovered essential workflow, or controlled material performance advantage. Revise one authoritative contract and dependent code, not a new parallel proposal. Do not start another broad research phase by default.
