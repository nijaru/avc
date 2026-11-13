# AVC implementation plan and release gates

**Updated 2026-11-12.** The architecture has been **selected and implementation has begun**. This is the single actionable plan for local development. There is no parallel research-first redesign track. Architecture [design](design.md); canonical bytes/conflicts [format](format.md); local durability [storage](storage.md); native sync [sync](sync.md); Git adapter [git](git.md); CLI [UX](ux.md); rationale [research](research.md).

**Present state:** a new Rust library and experimental source/history repository and CLI now implement the current format in part. Independent Python vectors and current-format tests replace the retired prototype; this is not a migration of published data. The obsolete codec, old tests and studies remain intact in [the pinned previous repository revision](https://github.com/nijaru/avc/tree/af66a63898f383b596e4828199dc625a80e28a72), not as current normative code. Their old State/provenance kinds, 4 KiB leaves, driver and exact IDs remain superseded. Basic authored commits, first-parent log, selected-history transfer, tip amend, scoped metadata undo and single-change rebase are implemented. A local SHA-1 Git adapter imports exact tips and exports candidate objects with retained origins. Explicit existing leaf and recursive directory capture are implemented; workspace recovery, remote synchronization and a sync service are not implemented.

### Implemented slices and next implementation

- **Stage 0:** new library/CLI, CI configured for macOS/Linux, independent v1 Python encoder/decoder and fresh canonical/malformed corpus. Current reference: 35 objects, 23 entry points, 50 negative corpora; Rust/Python cross-check generated maps in both directions.
- **Stage 1:** conditional path-copy radix edits, simultaneous witnessed move/copy/swap, cached verified bounds, source reads, threshold-safe Merkle diff and separate full contextual verification. Random edit orders/rebuild oracles and wide/shared-prefix cost probes exercise structural sharing. Heap-stack traversals replace the source/radix recursion that overflowed a 2 MiB debug stack. Bounded adversarial deep-radix and full-path fixtures now pass on that stack in debug and release; canonical path bounds were not lowered.
- **Stage 1A:** focused two-agent composition and source witness cases, signed rebase/revert counterexamples, conservative witnessed relocation composition, and a real local blobless Git-relative tree edit. Review found and corrected divergent-resolution false-clean cancellation, root-Absent legality and conflicted-content mode loss; [research §13](research.md#13-first-native-qualification-and-demonstrated-corrections-november-5-2026) records failed-before-fix evidence and limits. These are pure source tests, not a complete authored-history implementation.
- **Stage 2, initial vertical slice:** experimental `init/status/show/diff/verify/export/import/restore/receipt` and checkout-free `source` commands on one SQLite store (now physical schema 13: payloads, source/head selection and typed receipts share each transaction), with generation CAS, protected before/after source receipts, reopen, independently checked export/import, no-clobber fresh-directory materialization, concurrent writers, corrupt-object refusal and process termination around commit. Installation-sync and identical-request preparation races have failed-before-fix regressions. Explicit size limits and unimplemented physical/metadata features are in [storage](storage.md#current-implementation-boundary-november-2026). Reads do not scan/checkpoint the filesystem; source updates do not author revisions.

- **Stage 3, first slice:** explicit `commit` of selected source, random ChangeIds, immutable parent-linked Revisions, first-parent `log`, status with authored head/unborn state and typed idempotent receipts. Source and authored head share one CAS generation. Actual CLI tests independently verify generated revision ancestry with Python; retry/concurrent-writer/rollback tests cover publication. Source export remains source-only, not authored-history backup.

### Delivery order from the current implementation

1. **Portable authored history — implemented:** explicit `--history` exports the selected source and optional authored head with all owning ancestry; fresh-store import preserves exact IDs, parents and ChangeIds. Source-only transfer remains distinct.
2. **Basic history editing — implemented:** amend the selected tip, undo only the current generation's exact operation, and rebase one clean change onto an exact known revision. Retain old revisions and handle selected-but-uncommitted source explicitly.
3. **Small Git round trip — implemented:** import one exact Git tip, author natively, export a representable child and independently verify its Git source and parent. Native-born history can also export directly into an empty Git store. This can land before all of Stage 3; it does not depend on advanced restacking or filtered Git support.
4. **Ordinary filesystem capture — explicit leaf/directory and covered-removal subsets implemented:** file/symlink leaves and opt-in recursive directory scopes are pinned under the source CAS, with retained inclusion/removal policy, scope/witnesses, observed counts, removals and exclusions. `--recursive --remove-missing` removes old raw-name bindings only with complete parent-directory coverage, including directly selected missing paths without capturing sibling payloads; failed reads never imply absence. No live files are written and no revision is authored. Review exact roots before commit; cancellation uses scoped undo. Untracked missing requests, missing parents and directory-to-leaf replacement still refuse. In-place switching and automatic checkpoints remain separate lifecycles.
5. **Broaden supported workflows — local named refs and exact revision selection implemented:** independent per-name CAS and immutable scoped receipts retain/move known native revision aliases without changing selected source/head. Checkout-free `history select` selects an exact stored revision under the source/head CAS, refusing dirty selected source; commits can branch from that revision without implicitly advancing names. Upstream Git synchronization, independent work selections, richer history editing and foreign partial views remain ahead, followed by native partial clients and hosting.

The stage numbers below group concerns, not a requirement to finish every feature in one stage before starting the next. SQLite-only remains the one backend; no hardware-certification, GC, compression or hosting prerequisite is added. Further checks follow the features they protect. Stages 3 and 4 are partial, Stages 5–7 unimplemented, and the format is not frozen.

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

**Pure-engine gate:** exact source equivalence to an independent flat-path oracle across randomized operation sequences; order-independent canonical roots; typed invalid input rejection; no full-tree walk for a normal in-memory one-file edit. This does not establish incremental repository-command performance; that remains work under Stage 2.

## Stage 1A: Research resolved

**Research disposition is already recorded in [§12](research.md#12-evidence-first-resolution).** Git `merge-tree` and GitHub's Trees API already implement checkout-free source operations; Pijul records causal changes from ordinary edited working directories; Jujutsu documents signed conflicts and the same-change intent-loss counterexample. Published merge evaluations already contain **6,045 real-world merge scenarios**, and a 2026 study replays **747 agent-authored PR pairs**. **Do not reproduce these broad demonstrations as prerequisite feasibility studies.**

After the Stage 1 pure engine works, run only three **bounded** AVC checks:

1. **Exact source composition contract:** two agents propose conditional edits against one immutable source root; compare disjoint edits, conflicting edits, stale witnesses, move/delete and incomplete source proofs. Return verified roots or typed refusal. This is conformance, **not a claim of novelty over Git**. Preserve known typed operation witnesses where useful; do not invent a second canonical patch store.
2. **Conflict/change semantic counterexamples:** port a few cases from Jujutsu's documented same-change loss, Pijul's causal order/dependency examples and AVC's historical tests; include repeated rebase/revert, conflict resolution then rebase, binary/type/rename/delete. The signed-conflict v1 format is **not frozen** until these pass or the spec is corrected. Use the established [ASE 2024 evaluation corpus](https://github.com/benedikt-schesch/AST-Merging-Evaluation) for later broader merge-driver testing; no need to implement Pijul itself.
3. **Git boundary proof:** for a representative filtered Git tree, prove the foreign blob OIDs cannot be treated as native BLAKE3 IDs, validate a Git-relative edit using normal Git APIs, and specify the optional **shared server-side conversion/cache** alternative. Measure actual tip conversion and partial-client tradeoffs during Stage 4's real Git adapter, not as a speculative custom clone benchmark now.

**Gate:** the new engine passes these contract tests; no demonstrably unsafe conflict semantics or false native-root claim. Published research already supports the architecture direction; **do not make a large competitor benchmark a prerequisite for creating the durable local store**. Compare complete agent tasks against Git+jj/GitButler once AVC actually supports equivalent tasks. A concrete failing counterexample can still overturn unreleased source/conflict choices.

## Stage 2 — durable local repository and verified recovery

Implement [storage.md](storage.md): the SQLite WAL/FULL store (physical schema 13) owning payload rows, selection and receipts in one transaction, bounded closures, idempotent receipts, object verify/export/restore. Compression, segments/extents and streaming beyond the bounds are deferred to measured need. Add the actual CLI `init`, `status` (read-only), `show`, `diff`, `commit`, `log`, `verify`, and `export` on **one** source engine. Use no Git backend, daemon or mandatory service.

Fresh-directory materialization is implemented. When adding filesystem capture, pin selected bytes and report observation scope; when adding in-place switching, protect before-images and make interrupted effects recoverable. Checkpoints and private work handles need truthful retention, but are not prerequisites for authoring already-selected source.

**Fault coverage:** deterministic failures at SQLite begin/commit boundaries, quota/disk-full conditions, sync calls, ref CAS, backup and restore stages, and materialization rename/removal steps, verifying byte-perfect restore after crash for every acknowledged root. Test corrupt/unexpectedly missing object rows, denied mutation boundaries, concurrent reader/writer, and case/symlink escape. This is scoped integration fault testing of AVC's own code paths, not OS/hardware power-loss certification. The equivalent-work control comparison decided the physical backend ([research §14](research.md#14-sqlite-only-storage-decision-and-equivalent-work-control-november-6-2026)); there is exactly one store, and no second physical backend is maintained.

**Implemented acceptance:** independent offline source export/import/restore and explicit selected-history transfer. Default source export still omits Revisions and the authored head; `--history` includes them, without claiming all local operations or workstation state. Commands remain `--experimental` while the format is unreleased.

**Repository incrementality remains incomplete:** source-edit/commit paths still load/verify complete source closures. Add store-backed object access and reuse of validated unchanged subtrees under [storage §4](storage.md#4-verification-closure-and-physical-identity), without another backend. Measure one-file edits through the real CLI; keep explicit full verification and untrusted-import validation. Separate per-object, source-operation, metadata-page and history-transfer budgets: current closure limits must not become a lifetime repository-size rule. Initial feature slices may retain declared aggregate limits and refuse oversized work, never truncate a purported complete result.

## Stage 3 — authoring, history and first-class conflicts

**Implemented:** initial authoring, immutable parent-linked ancestry, random ChangeIds, selection CAS, typed retry receipts, first-parent log, selected-history transfer, tip amend, scoped metadata undo and single-change rebase. Deliver the remaining capabilities in small slices:

### Portable authored history — implemented

Explicit `export --history` / `import --history` implements [storage §8](storage.md#8-export-backup-and-disaster-recovery) with unchanged native-only `AVCHIS01` framing and `AVCHIS02` for retained Git origins. Canonical Revision IDs, ordered parents, ChangeIds, metadata and selected-but-uncommitted source survive fresh-store restore. All ordered parents are owned, not just the first-parent log.

**Verified acceptance:** actual CLI export restores and re-exports identically after removal of the original store; an independent Python parser verifies its bytes. Corruption, missing non-first parents, surplus objects and incomplete streams refuse without publishing. Concurrent/retried imports and denied receipt insertion cover the shared publication boundary. Scope remains selected history and retained foreign boundaries, not every receipt, private root, derived Git mapping or workstation setting.

### Basic history editing

**Implemented:** tip amend and scoped metadata undo. Amend authors selected source with the same ChangeId and exact ordered parents, retaining the predecessor. Undo restores the exact current operation's before-source/head under a new generation and retry identity; it never changes live files/remotes. Tests cover imported merge parents, dirty selected source, empty/unchanged outcomes, retries, concurrency, rollback and corruption. Review found and fixed unchecked new-parent metadata and post-publication diagnostics hiding a recovered receipt; both have failed-before-fix regressions.

**Implemented:** single-change rebase against fully known ancestry. It preserves authored metadata and ChangeId, uses the exact original parent as comparison (empty source if parentless), and makes the exact target its sole new parent. Dirty selected source, multiple original parents and target ancestry containing the same ChangeId refuse. No-op rebase retains the RevisionId with a new generation/receipt; genuine source conflicts remain typed and are reported. Follow [UX §3](ux.md#3-exact-authoring-and-change-editing) for selected-source behavior and [storage §7](storage.md#7-operation-journal-and-retryundo) for publication/undo. Preserve ChangeId across rewrites, retain exact predecessor revisions, and commit new objects, selection and typed receipt together. Undo is a new generation-checked operation, never a rewind of generation counters or a filesystem restore.

**Verified rebase acceptance:** disjoint source composition, binary conflict terms, rootless replay, exact modes/symlinks/empty directories, authored metadata, unchanged/empty outcomes, retry/undo, concurrent CAS, denied-receipt rollback and corrupt-input refusal are covered. Differing resolutions of a conflicted base refuse; equal-resolution fast paths remain safe. Independent Python verifies actual CLI history exports. The first small Git round trip, explicit leaf/directory capture and covered missing-binding removal are implemented; advanced restacking remains separate. Keep `UnresolvedBase` where required: refusal is preferable to false-clean source loss. Do not claim arbitrary conflicted-history rebase is solved by the existing pure-source tests.

### Ordinary filesystem capture

After the initial Git round trip, add the ordinary edit/select/commit workflow using [storage §6](storage.md#6-filesystem-observation-and-materialization) and [UX §3](ux.md#3-exact-authoring-and-change-editing). Capture bytes once, preview/select that pinned candidate, report omitted/unreadable/raced paths and never infer deletion from a failed read. Capture alone need not implement in-place checkout, background checkpoints or a full workspace manager.

**Implemented existing-directory subset:** `--recursive` explicitly includes new files and empty directories beneath selected paths; `.` selects the whole root. Add `--remove-missing` to remove old raw-name bindings absent from a complete successful directory enumeration. Retained raw name lists and directory witnesses distinguish covered absence from read errors. Directly requested tracked missing files, links and directories use membership-only parent coverage, leaving unselected sibling payloads unchanged. Untracked missing requests, missing parents and directory-to-leaf replacements still refuse. Administration source bindings are preserved, and pruning an absent ancestor containing protected bindings or unresolved conflicts refuses. Inclusion/removal policy participates in retry identity, including captures with no removals and recovery after the original files disappear. Optional report failure cannot hide a recovered receipt.

**Acceptance:** later file edits or recreations cannot silently enter a commit of captured bytes/absence; cancellation authors nothing and restores selected source rather than live files; unselected source stays untouched. Failed reads and protected-subtree removal refuse without partial publication. Observation scope is not represented as an atomic filesystem snapshot.

### Later history capabilities

**Implemented:** local named revision references with independent per-name generations, shared request-key reconciliation and typed ref receipts. The append-only operation journal owns each transition and current value; ref set leaves selected source/head and its generation unchanged. Metadata-only target checks avoid imposing a lifetime owning-history limit. Names do not follow commits/rewrites automatically. Checkout, ref deletion/rename/undo, reference transfer and remote publication remain unimplemented ([storage §2](storage.md#2-sqlite-authority-and-local-record-model)).

**Implemented:** checkout-free exact revision selection through `history select`, with clean-selected-source guards, complete destination-current-source verification, global retry identity and generation-checked undo. Selection creates no objects and changes no files or named refs; target history and retained Git origins are not recertified. This is detached metadata selection, not current-branch tracking or filesystem switching.

Independent work selections still need scoped generations, not aliases for the singleton source/head selection. Add descendant restacking, merge-parent authoring, exact split/fold/divergence lineage and richer resolution as their workflows are implemented. `absorb` and stack composition remain Stage 6, not prerequisites for Git onboarding.

Use Stage 1A's exact counterexamples for known-base merges, ambiguous ancestry, move/path collisions, binary/type/symlink transitions and repeated conflicted rebase. Unknown ancestry and resource refusal must not become fabricated parents, clean values or semantic conflicts. Deletion/GC retention checks arrive only if deletion is implemented.

## Stage 4 — Git onboarding early

Use mature Git subprocess/library support and the boundary in [git.md](git.md), not a new pack protocol. Split ordinary onboarding from massive filtered repositories:

### Small local round trip

**Implemented local subset:** import a complete selected SHA-1 tip from an isolated small Git repository, create the deterministic native anchor and durable foreign-origin mapping, author a native child, then export a Git child of the exact original Git tip. Native-born parentless history also exports directly as Git roots, including after native history transfer. Use a declared representable subset and report unsupported source/metadata before publication. No network hosting, sophisticated rewrite UX or foreign partial mode is needed for this slice.

**Acceptance:** two imports of the same supported Git commit/source converge to the same anchor; stock Git independently verifies the exported bytes, executable/symlink modes and parent. Repeated translation does not duplicate imports or advance an unrelated ref. The implemented export produces candidate objects only; it does not publish refs or use the network. Known import anchors with missing origins refuse rather than being reclassified as native roots. Git-anchored selected history uses `AVCHIS02` to retain exact origin metadata and restore offline. Exact current-source origin objects can be rehydrated into an empty Git target; foreign ancestor bodies remain opaque frontiers. Complete origin declarations bind import retries and protect restored anchors from native-root fallback after metadata damage. Exact metadata/time projection and source refusals are owned by [Git](git.md#available-local-round-trip).

### Ordinary remote collaboration

Add guarded tracking/fetch/push and lost-response reconciliation. Then exercise upstream advancement and a safe fetch/rebase/push cycle. Refuse stale leases and disclose foreign ancestry; never invent a native merge base across an unknown frontier.

**Acceptance:** an isolated Git remote round trip preserves bytes and ancestry; an upstream race or unknown push response cannot silently overwrite remote work or duplicate publication.

### Foreign partial views and broader compatibility

Add explicitly foreign filtered views for huge Git sources separately. Reuse unchanged Git OIDs without claiming native BLAKE3 IDs for unfetched bytes; hydration/conversion stays explicit. Cover both Git hash algorithms, merge ancestry and other representability cases as support is added; unsupported cases must remain explicit refusals.

A blobless foreign agent should eventually edit/push without a false native-root claim. The optional shared Git→AVC conversion cache remains deferred to demonstrated repeated conversion cost or hosted demand; it is not needed for ordinary local Git onboarding.

## Stage 5 — native partial-clone and checkout-free agent API

Implement [sync.md](sync.md): exact ref resolution, explicit object availability/promisor states, authenticated Merkle path proofs, path-scoped conditional root edits, grouped/resumable native transport, offline/full hydration, and remote ref CAS with durable object closure and idempotent retries. The remote service uses the same canonical library operations; do not add a second server-side source authority.

**Gate:** large native repo scoped/no-checkout read→edit→candidate root using sibling proof→server completes closure→two agents race to publish→one succeeds/other gets `StaleRef`→safe rebase/retry; offline full hydrate/export works while partial clone reports missing data precisely.

## Stage 6 — composed changes and hosted scale

Extend the Stage 1A **minimal derived composition API** into optional GitButler-style independent/dependent local stacks and reproducible composed source views, explicit hunk assignment, safe `absorb` preview and candidate review/test references. Verify isolation modes: distinct directories, shared directory under cooperative use, and checkout-free conditional agents; measure setup versus collisions and hidden dependency effects.

Only then profile hosting throughput and implement derived path history, blame, rename hints, cache tiers, reusable transfer packs, Zstandard dictionaries and physical compaction policy where justified. None of these changes source IDs. Hosted features, Canto integration, CIs and forges are separate products/clients.

## Stage 7 — optional Git protocol frontend on native hosting

**Only after** native local storage and recovery, basic Git remote bridge and native hosted CAS are reliable. This is a *compatibility interface*, **not** another Git-backed AVC storage engine.

1. Implement deterministic derived Git blob/tree/commit projection and a versioned mapping/cache for Git-visible **representable** native revision closures; refuse conflict/empty-directory/unrepresentable/incomplete histories unless the user explicitly selects a separate projection.
2. Implement Git protocol v2 fetch and ref advertisement backed by the **same authoritative native ref generation**, not an independently advancing shadow Git ref. Validate with stock Git clients and standard pack-object/reachable-object rules.
3. Implement Git receive/push translation: validate received Git OIDs/pack/commit ancestry and permissions, convert complete supported inputs to native revisions, enforce native CAS plus Git old-OID/fast-forward or explicit force-lease conditions, then durably acknowledge. Test races between Git and native publishers and response loss after commit.
4. Add shallow/deepen, partial-clone filtering, tags and physical cache improvements only when compatibility demand justifies maintaining each capability.

**Gate:** native and Git readers see the same representable shared ref; concurrent Git/native pushes cannot lose a committed update; native-only states never masquerade as clean Git history. A Git client can fetch/push supported source without AVC storage becoming Git-backed. Unsupported source or missing history produces clear refusal and recovery guidance.

## Release standards and evidence

Before declaring the supported format and workflows stable, require the checks below for that declared feature set. Later partial-client, hosted, GC and in-place-switching features bring their own checks; they are not prerequisites for delivering the earlier local/Git slices.

- Independent codec vectors plus malformed inputs, fuzz and property oracles, including signed expression normalization, branch split/collapse and revision ancestry.
- Durable complete-root acknowledgement and exact backup/restore under the declared SQLite WAL/FULL plus scoped syscall/process-crash model, not device certification. Interrupted switching and GC safety join when overwrite and deletion are supported.
- Explicit partial/foreign/missing/unauthorized states when those modes are supported; authenticated membership/absence and sibling-name confidentiality when restricted clients are supported.
- **Comparative product evidence:** use existing research, then compare representative supported human/agent tasks against Git+Jujutsu and relevant stack/causal-edit tools where semantics match. Cover ordinary Git collaboration, path-scoped agents, concurrent edits/restacks and recovery as each workflow becomes available—not a blanket prerequisite to implement all of them first. Measure complete-task latency, resource use, conflict burden and recovery under matched cold/warm conditions before claiming comparative gains. Judge foreign partial Git separately from native AVC. See the [competitive falsification record](research.md#10-november-4-competitor-steelman-after-the-one-native-backend-decision); an incremental Merkle microbenchmark alone does not demonstrate product advantage.
- Git interoperability with documented supported subset and exact refusal of lossy states; no silent move of foreign or public refs.
- No implicit execution of untrusted source hooks or unsafe symlink/path traversal.

**The core architecture is selected; the early Stage 1A experiment is a targeted product/semantic falsification gate, not a second architecture project.** There are no unresolved architecture choices requiring user input. Any future design change must cite a concrete failed invariant, uncovered essential workflow, or controlled material performance advantage. Revise one authoritative contract and dependent code, not a new parallel proposal. Do not start another broad research phase by default.
