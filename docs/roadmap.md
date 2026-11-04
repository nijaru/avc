# AVC implementation plan and release gates

**2026-11-04.** The architecture has been **selected and written as implementation contracts**. This is the single actionable plan for the upcoming **local device** session. There is no parallel research-first redesign track. Architecture [design](design.md); canonical bytes/conflicts [format](format.md); local custody [storage](storage.md); native sync [sync](sync.md); Git adapter [git](git.md); CLI [UX](ux.md); rationale [research](research.md).

**Present state:** design-only active tree after deliberate retirement of the obsolete native codec, old format tests, independent old-format experiments and their CI. They are available intact in [the pinned previous repository revision](https://github.com/nijaru/avc/tree/af66a63898f383b596e4828199dc625a80e28a72). Do not treat their exact IDs, old State/provenance kinds, 4 KiB nibble leaves, old text driver or 151-test reference suite as current normative behavior. No native repository commands or source custody currently exist. The next phase starts a **new Rust implementation**, not a migration of a published format.

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

## Stage 2 — durable local repository and safe source custody

Implement [storage.md](storage.md): SQLite WAL/FULL metadata authority, staged immutable object segments, bounded streaming, transactionally installed roots and work refs, reader/GC protection, idempotent receipts, object verify/export/restore. Add the actual CLI `init`, `status` (read-only), `show`, `diff`, `commit`, `log`, `verify`, and `export` on **one** source engine. Use no Git backend, daemon or mandatory service.

Implement exact scoped filesystem capture with pinned selected bytes; preflight path/symlink and stale-write guards; guard non-atomic filesystem materialization with before-image protection and recoverable operation receipts. Default recovery into a new directory. Add explicit checkpoint with truthful scope/retention and private work handles before history convenience.

**Mandatory fault matrix:** fail at each file write, file/directory sync, stage install, SQLite begin/commit, ref CAS, reader lease, compaction location flip, GC retirement, backup, and materialization rename/removal. For every acknowledged root verify byte-perfect restore after crash. Test corrupt/unexpectedly missing blobs, quota exhaustion, concurrent reader/writer, and case/symlink escape. Compare a SQLite-only control **for identical safety tests**; retain the selected hybrid backend only if it passes and justifies its extra physical complexity.

**Gate:** full native offline recovery independent of any server. Until this passes, operational commands must be labeled experimental or refuse important-source custody; the repository has no claim to protect irreplaceable work.

## Stage 3 — authoring, history and first-class conflicts

Implement immutable Revision DAG, random stable ChangeId, selection/ref CAS, amend/rebase/restack, exact lineage and scoped local operation-log undo. Implement recursive known-base merge and the selected normalized signed conflict objects, with lossless alternate binary/path/type/source values, bounded deterministic text driver, resolution, revert and repeated conflict rebase.

Test incomplete/promised ancestry, **typed `AmbiguousMergeBase` refusal and explicit verified base selection** on multiple maximal bases, simultaneous conditional subtree moves and path-witness collisions, partial path edits, overlapping same-file hunks, symlink and directory transitions, merge of conflict-valued source, `absorb` suggestions on clearly attributable versus ambiguous hunks, and resource-failure-as-pending (not fabricated conflict).

**Gate:** repeated history rewrite and merge cannot silently discard source alternatives or invent parents; exact revision selection remains recoverable across crash and GC.

## Stage 4 — Git onboarding early

Implement [git.md](git.md) against mature Git client/library code, not a new Git pack protocol: exact selected-tip source conversion to native, **deterministic import anchor identical across two clients given the same Git commit/source**, foreign ancestry frontier, remote tracking, safe Git commit/tree export, push with lease and lost-response reconciliation. Validate with *actual Git* on a real small repository, not just AVC oracles.

Provide an **explicit** foreign partial view for massive filtered Git repos, including Git-relative path queries and conditional Git-native commits/push that reuse unchanged Git object IDs without full native import. Do not claim source-specific BLAKE3 IDs for unfetched blobs. Full native import from foreign view is a separately invoked hydrate/convert action.

**Gate:** Git clone→native edit→native commit→Git push→independent Git byte/parent verification→upstream change→safe fetch/rebase/push; a blobless foreign agent can do Git-relative work without a false native root.

## Stage 5 — native partial-clone and checkout-free agent API

Implement [sync.md](sync.md): exact ref resolution, explicit object availability/promisor states, authenticated Merkle path proofs, path-scoped conditional root edits, grouped/resumable native transport, offline/full hydration, and remote ref CAS with durable object closure and idempotent retries. The remote service uses the same canonical library operations; do not add a second server-side source authority.

**Gate:** large native repo scoped/no-checkout read→edit→candidate root using sibling proof→server completes closure→two agents race to publish→one succeeds/other gets `StaleRef`→safe rebase/retry; offline full hydrate/export works while partial clone reports missing data precisely.

## Stage 6 — composed changes and hosted scale

Add optional GitButler-style independent/dependent local stacks and reproducible composed source view, explicit hunk assignment, safe `absorb` preview and candidate review/test references. Verify isolation modes: distinct directories, shared directory under cooperative use, and checkout-free conditional agents; measure setup versus collisions and hidden dependency effects.

Only then profile hosting throughput and implement derived path history, blame, rename hints, cache tiers, reusable transfer packs, Zstandard dictionaries and physical compaction policy where justified. None of these changes source IDs. Hosted features, Canto integration, CIs and forges are separate products/clients.

## Release standards and evidence

Before public stable bytes or important-source custody, require:

- Independent codec vectors plus malformed inputs, fuzz and property oracles, including signed expression normalization, branch split/collapse and revision ancestry.
- Durable complete-root acknowledgement, GC safety, exact backup/restore and interrupted file switching on supported macOS/Linux filesystems; no source loss under declared failure model.
- Explicit partial/foreign/missing/unauthorized states; authenticated membership/absence and no sibling-name leakage for restricted clients.
- Representative human and agent tasks against Git/Jujutsu where semantics match, cold/warm cache, SSD/RAM/CPU/network bytes, p50/p95, later history deepening and change-conflict burden.
- Git interoperability with documented supported subset and exact refusal of lossy states; no silent move of foreign or public refs.
- No implicit execution of untrusted source hooks or unsafe symlink/path traversal.

**The design phase is complete enough to begin.** There are no unresolved architecture choices requiring user input. Any future design change must cite a concrete failed invariant, uncovered essential workflow, or controlled material performance advantage. Revise one authoritative contract and dependent code, not a new parallel proposal. Do not start another broad research phase by default.
