# AVC — native source control architecture

**Design decision: November 3, 2026 (independent steelman).** This is the **authoritative design**, sufficiently defined to implement a first native vertical slice, **not** a frozen byte format or verified production engine. Earlier proposals are superseded; the October/November 2 and first November 3 contracts remain available in [Git history](https://github.com/nijaru/avc/blob/94dd9921ee8a93b89551cbecf53bb14f16f08b01/docs/design.md). The existing experimental Rust codec implements the *older* proposal and is not a compatibility constraint. The [workflow contract](ux.md), [qualification plan](roadmap.md), and [research/decisions](research.md) have separate ownership.

## Product and decision rule

AVC is a **local-first, native, general-purpose VCS** with reliable offline source ownership, cheap source queries without checkout, editable history and recoverable work, scalable selective fetching, and excellent **Git adoption without Git authority**. It serves ordinary local development and terminals, persistent SSH work, CI, ephemeral agents, and optional hosted large repositories.

Optimize **complete developer/agent tasks** (clone → inspect → edit → test → preserve → review → integrate) at real resource costs, not theoretical minimum commit size, novel algebra, or a claimed numerical optimum across every workload. Favor correctness, observability, maintainability, and end-to-end performance. Cross-company upstream speedups are not AVC benchmarks. Detailed implementation constants can change before release if tested; do **not** leave core semantics indefinitely undecided waiting for arbitrary research.

A full local repository needs no server, daemon, mount, or agent harness. A thin repository has explicit completeness limits and may rely on a remote. Git is an adapter, not the canonical object store or synchronization protocol.

### Non-negotiable invariants

1. **Exact source:** A complete, admitted source reference reconstructs one exact typed namespace, independent of its author, ancestry, compression, materialization, and caches.
2. **Single source authority:** The canonical source reference is the persistent Merkle source value. Changed-path descriptions and diffs are optional *authenticated construction hints*, not a second history whose replay might disagree.
3. **Honest custody:** Acknowledging a protected local root, checkpoint, export, or remote ref requires the entire *promised durability closure* to be present and durable within the declared retention/failure domain. No incomplete promise is advertised as full custody.
4. **Safe uncertainty:** Unsupported, conflicting, incomplete, inaccessible, or temporarily unavailable input is never silently treated as absent source or a successful clean merge.
5. **Independent work:** Different workspaces and local change candidates advance independently; advancing a shared ref is conditional. User-visible work is not inferred from filesystem location or equal content.
6. **No false atomic filesystem snapshot:** External tools may modify normal directories during observation. AVC records the exact bytes it actually captured and uses guards before overwriting paths.
7. **No implicit execution or disclosure:** Reading/parsing/importing source never runs its hooks or agent tools. Sharing unpublished source, checkpoints, and evidence requires a distinct operation.
8. **Import honesty:** Git or any foreign repository has a distinct foreign identity. AVC cannot claim a complete native BLAKE3 source root from unfetched foreign blob identifiers.
9. **Derived indexes:** Rebuilding indexes may be expensive but cannot change source semantics, remove promised data, or turn unverifiable input into a known answer.

## 1. Canonical logical model: choose snapshots, incremental construction

**Selected model: content-addressed persistent source trees directly referenced by immutable authored revisions.** A canonical changelog is **not** an additional source authority.

The minimal immutable logical objects are:

- **Blob**: exact regular-file bytes or symlink-target bytes, identified by typed, domain-separated BLAKE3-256 hash over canonical uncompressed bytes.
- **Tree nodes**: deterministic, immutable directory bindings over raw filename bytes. Entries are file {BlobId, executable}, symlink {BlobId}, directory {TreeId}, or unresolved typed source value. Empty directories are representable; special files, ACLs, xattrs, case-folding, mtime and inode identity are not silently normalized into source.
- **Conflict values**: unresolved typed alternatives that preserve the affected source entries/trees and comparison context. Conflicts are source data, not marker text or a completed semantic merge.
- **Revision**: immutable source root, ordered revision parents (when complete), logical ChangeId, authored metadata. A revision's source root may itself be an unresolved directory-valued source. Revision ancestry is distinct from source identity.

**Do not require the separate v0 State wrapper** merely to hold a directory root. Use a kind-checked root reference; a conflict-valued root is still explicit. A new clean revision can reuse the same exact source root as another author/history.

**Stable ChangeId** is a random logical handle preserved by a straightforward amend/rebase. New authored changes receive new identities. Rewrites produce new immutable RevisionIds and optional predecessor/successor *metadata*; split/fold and divergent successors have explicit many-to-many mappings, never a guessed unique latest revision. A review or test attestation binds the **exact source/revision plus its relevant comparison/execution scope**, not a floating change name. Detached signatures and optional evidence do not affect source identity and do not make unpublished inputs durable.

**Ordinary commit construction is incremental.** For a batch of conditional path updates from a verified base tree, path-copy only affected directory-map nodes, reuse verified immutable siblings, and generate a new root. Target cost is proportional to changed paths and directory-tree depth plus touched node bytes; no full source-tree rebuild or full history replay on every edit. Changed-path summaries may be stored alongside a revision to make authoring, history queries and transport cheap, but every claimed result must validate against the actual source root.

This selection rejects canonical causal/commuting patches as a universal authority: arbitrary binary edits, filesystem writers, directory moves, and ambiguous edit intent have no guaranteed causal source operation. Allow **optional** semantically richer change descriptions and composition algorithms when provenance is known; always bind their result to an exact source tree. A canonical changeset-first/derived-manifest system (Scion/Mononoke) could outperform snapshot roots on an unusual write-heavy, seldom-read hosted workload, but does not establish a better general-purpose offline system absent equivalent measurements. See [research](research.md).

### Canonical directory map choice

**Select a hierarchical, path-compressed radix map with compact, sorted leaves as the implementation baseline.** The earlier bounded study showed materially smaller cold reads and update bytes than flat maps; one-key leaf maps inflated object counts. A radix map offers deterministic prefix-local edits, subtree reuse, exact raw names, efficient path and range queries, and bounded fanout. This is *not* a proof that the previous nibble codec or 4096-byte leaf threshold is optimal.

Build a real **persistent update** path, not the prototype's full FlatMap → build() on each change. Use a small sorted leaf when the entire directory fits; split a wide directory deterministically on raw-name prefixes into bounded radix nodes. Canonical construction must be insertion-order independent, including split/collapse thresholds. Compare the production candidate once against a deterministic chunked ordered-map alternative using real edit/move/lookup and memory costs. Freeze actual byte grammar and thresholds **after** that qualification, not before. Prefer an unambiguous codec and simple incremental verifier over marginal theoretical compression.

Path rules must protect exact byte names, prohibit NUL, separators, \`.\` and \`..\`, reject ambiguous duplicate/case-colliding materialization, and refuse unsafe traversal or symlink redirection. Logical source may be valid even when a particular filesystem cannot faithfully materialize it. Include bounded depth, encoded-length and input allocation checks.

### Verification without whole-tree work on every commit

At untrusted admission: validate object envelope/hash, typed canonical decoding, local node ordering/bounds, and authorized dependencies. Newly built nodes are validated against a trusted verified base and exact conditional witnesses. An unchanged previously verified immutable subtree may be reused without walking every descendant. Cache verification status by exact ID, codec version, and required context/bounds; never trust a copied or unauthenticated cache summary. Full-closure audits, import verification, backup/restore qualification and periodic integrity scans are **separate, potentially expensive operations**. Validation cache corruption must not yield false durable claims.

## 2. History, operations, and working state

Separate immutable authored history from mutable local **repository view**:

- **Revision DAG** owns authored parents and source identity.
- **Local view** owns workspace selections, draft/change-stack heads, native and foreign remote tracking refs, retained checkpoints, and protected object roots.
- **Durable operation journal** records transactional changes to that local view and its expected generation, with enough before/after information for scoped undo and interrupted-operation recovery; it is **not another canonical source DAG**.
- **Filesystem observation** supplies checked candidate bytes, not a globally atomic directory truth.
- **Publication** conditionally updates a remote ref only after required objects are durably available.

Basic commands remain familiar (\`status\`, \`diff\`, \`show\`, \`commit\`, \`log\`, \`switch\`, \`merge\`, \`rebase\`). Selective file/hunk commit is based on pinned captured bytes, with no mandatory mutable Git staging index. The exact source candidate shown in a preview is the candidate authored; editor writes after preview remain outside it. Initial uncommitted work must be recoverable before the first authored revision; do not manufacture canonical task/effort commits merely to make navigation work.

**Private recovery is a protected source checkpoint** with recorded base, captured tracked/untracked scope, omissions, explicit ignored-file policy, and retention. It does not become a commit, publish or silently disclose new files. AVC must protect observed work before performing a destructive switch/restore and use before-image guards; if a writer changes a file after observation, do not clobber it. A reader-only status is permitted to be a hint and must not imply recovery coverage or atomic observation. Unsaved editor buffers/process/VM state are outside AVC's custody.

Use short transactionally durable view changes and idempotent receipts. History operation undo changes selected metadata and retention; filesystem rollback/recovery is a separately guarded action, so undo must not pretend to revert arbitrary editor writes. Do not keep every old source object alive forever just because an evolution relationship or review metadata names its ID.

## 3. Merge and change composition

**The source merge default is deterministic three-way value merging**, with exact explicit base(s), fast equality checks, recursive directory combination when types permit, and a versioned practical text driver. Refuse incomplete ancestry rather than invent a merge base.

First-class conflicts preserve alternatives for binary, symlink, executable-mode, file/directory, delete/modify and text cases. Never silently convert unresolved source to clean Git-style marker bytes. A resource/remote/I/O failure yields a **pending/aborted operation retaining inputs**, not a fabricated source conflict.

**Select a simple lossless, typed, path-local conflict representation to start**; normalize duplicate exact alternatives and bound depth/size, but do not adopt a universal signed-term cancellation law or silently discard original terms without a proof of equivalence. The older nested-conflict prototype and all-optimal-alignment text driver are **not** automatically retained. Before canonical format freeze, qualify a compact flattened expression against the simple representation on repeated rebase, merges of conflicts, revert, delete/rename/type and large-file cases; choose the simpler one that preserves all alternatives. A tree-level merge expression is a real alternative, not a mandatory complexity tax. A merge that cannot safely represent all source alternatives may retain protected pending inputs and refuse a result.

Use standard well-tested text-merge strategies as baselines. Measure false-clean combinations and excess conflicts with real histories and adversarial cases; neither most-clean nor most-conservative wins by itself. Do not confuse deterministic results with semantic correctness.

**Composition is not commutativity.** Rebase, cherry-pick, restack, move, and preview are distinct transformations whose results may depend on ordering and historical base. Retain recorded source provenance where it makes these operations safer, but inferred matching bytes do not prove rename intent. A copied subtree can reuse its ID. A moved subtree also reuses its ID when path/shape constraints permit; context-dependent bounds still require validation or a verified summary.

## 4. Multiple independent and stacked changes

**Select GitButler-like change stacks as a local/agent UX feature built on revisions, not another authoritative graph of source edits.** Each stack owns a particular base and ordered change versions; dependencies are explicit. A **composed view** is an exact, reproducible source candidate produced by applying a declared order and merge/rebase rules. It is not automatically an authored merge commit or the permanent tip of every included change. Pin the candidate for review and tests.

A human may use one composed directory with explicit current edit destination; split/select hunks when assigning edits to separate changes. \`absorb\` may suggest moving later edits to an earlier change when line ancestry is unambiguous, with a before/after preview and full dependent restack; otherwise do nothing. Neither status-watchers nor timestamp guesses confer semantic hunk ownership.

Independent agents can use separate writable directories, shared runtime directories under **explicitly accepted coordination risk**, or checkout-free **conditional source APIs**. An immutable candidate prevents source history races, but does not isolate filesystem writers, generated outputs, service ports, caches, databases or credentials. Standard API shape: \`apply(base_root, [{path, expected_old_entry_or_absent, new_entry}, ...]) -> verified new_root | structured stale/conflict/need_data\`. A shared branch update is a separate permissioned CAS. No daemon or agent-runtime-specific protocol is required for the library.

## 5. Native clones and source availability

Native clone defaults to **tip-first and expandable**: resolve the selected default branch/ref (or explicit revision), transfer enough current source for a usable ordinary directory, and avoid historical blobs that are not needed. Current branch **tip**, history horizon, available objects, and on-disk materialized paths are **four independent policies**.

- Normal developer clone: selected tip and practical full current checkout, with bounded recent history metadata and historical content fetched when needed. Display completeness. Do not silently omit files for a typical project.
- Large monorepo/sparse mode: explicit paths, checkout-free \`show\`/\`diff\`/source APIs, batched on-demand fetch, optional virtual filesystem only if available, and no cost of unnecessary current blobs.
- Ephemeral agent: allow \`--no-checkout\`, path scope, and short-lived draft roots uploaded before compute disappears.
- Full/offline mode: hydrate the **entire explicitly selected protected closure** (refs, revisions, trees, blobs), verify it, and export/backup independently. A path-only clone is not a full offline repository.

A native server can give an authenticated complete root and path-scoped proofs or perform server-side conditional construction while the client holds only selected objects. **A partial client cannot locally certify a full new root merely from unresolved promised children**. Fetch grouped objects and history metadata rather than blindly performing Git-style depth-one shallow operations; unknown ancestry remains a promised frontier, not an empty history.

Maintain typed state for \`present+verified\`, \`promised\`, \`unavailable/offline\`, \`unauthorized\`, \`absent proven by source\`, and \`corrupt\`. Requests may *read-through-fetch and fill an immutable local cache* under declared online policy; that does not make a read-only command an authored history mutation. \`status\` is local by default; \`show\`/\`diff\` may fetch missing promised data when online unless \`--offline\` is specified or a privacy policy forbids it. Never execute remote source as part of retrieval.

The object protocol supports exact immutable IDs, bounded batches, byte/range access, verified resumable downloads, prioritized paths, request coalescing, backpressure, and optional reusable transport packs. Keep *logical* BLAKE3 IDs stable when repacking/recoding physical data.

### Critical Git partial-import boundary

**Native BLAKE3 BlobIds cannot be derived from Git SHA-1/SHA-256 IDs without reading the content.** Therefore an arbitrary blobless/sparse Git checkout **cannot** be silently declared to have a complete native \`TreeId\`.

Two honest adoption modes:

1. **Native import (initial writable baseline):** fetch and verify the selected Git tip's *complete current source closure*; compute native IDs/trees, bind the exact Git tip as a foreign origin, and permit native edits. Historic Git parents and older blobs can be represented as a **clearly marked, unresolved foreign ancestry frontier** until converted; never treat the boundary as an actual root commit. Native history from this exact anchor remains stable while the adapter resolves earlier Git history.
2. **Foreign partial view (large Git monorepos):** retain Git IDs and the Git server's filtering/promisor semantics in a **separate adapter view**; read/inspect/prepare conditional Git-relative edits without asserting a native source root. Convert the complete requested source and history closure before authoring a full native revision, or apply an explicitly Git-native change through the adapter with Git's own preconditions and truthfully label it *foreign*, not native AVC history.

This is a deliberate v0 limitation, better than importing Git identifiers as competing canonical native blob IDs and thereby violating equal-source identity. A future universal external-ID or lazy foreign-base source representation needs an explicit design and compatibility evaluation, not implicit dual authority. **Git adoption can still be early and useful** for ordinary repos while the large foreign-Git bridge is investigated. Never claim transparent instant conversion of giant filtered Git repositories before it works.

Git import/export preserves exact representable file bytes, symlinks, modes, authored ancestry and available metadata with persistent foreign/native ID mappings and conditional ref updates. Unsupported empty directories, unresolved conflicts, unusual path/mode metadata, submodules, Git LFS policies, signed commits/tags, rewrites and incomplete history require precise behavior and loss disclosure. Git push is allowed only when the intended exported source is representable and target refs are still expected; never imply that new Git commits preserve old signed commit IDs. Use existing Git interoperability tools at the boundary before rebuilding Git protocol from scratch. Do not permit two independent authorities to race writes to the same local working directory by default.

## 6. Physical storage, transactions, and backup

**Selected local physical design: one SQLite metadata/root authority plus immutable staged payload segments**, both owned by a local Rust engine; no daemon, cloud database, repository-wide lock across read-only operations, or Git packfile requirement. SQLite maintains refs, root/pin generations, object locations, verified provenance of staged payloads, reader/GC epochs, and operation receipts. Immutable segments contain bounded object records, physically Zstandard-compressed or stored raw when compression hurts; larger blobs may be physically chunked and fetched by range. Object identity is over reconstructed *uncompressed logical bytes*, not chunking, dictionary, or compression choices.

The critical acknowledgement protocol:

1. Capture immutable bytes from explicit scope; check typed object hashes, canonical decoding, dependencies and limits.
2. Write newly needed segment data to staged files; check full segment checksum/contents and durably sync new data. Establish stable immutable names/locations with proper file and directory synchronization.
3. In a **short SQLite transaction** (WAL, FULL durability where the platform's VFS supports it), install object locations, exact protected root and receipt under a compare-and-swap of the expected local generation.
4. Report success **only after the durable root transaction succeeds**. Failed/interrupted preparation can leave harmless unreachable staged objects, never a durable acknowledged root referring to missing bytes.
5. GC and compaction use protected-root closure and reader leases; write new immutable segments before atomically updating locations, and retire old segments only after no reader, backup or retained root can require them.

Crash windows, filesystem sync semantics and SQLite WAL behavior must be qualified separately on macOS/Linux (and later other platforms). The old SQLite-only model is the **safety control** in bounded comparison; prefer it if the hybrid cannot match its reliability or performance in representative source custody. This choice is physical, not canonical: future backends may use object storage and distributed metadata while preserving the exact same source objects.

**BLAKE3** is the logical ID candidate, with type/version separation and independent codec vectors. **Zstandard** is a physical codec candidate, not a claim of universal advantage over Git's cross-file delta packs; benchmark complete fetch/checkout, random read, compression, CPU, tail latency, and storage amplification. Dictionaries, cross-object deltas and Pando-like LSM compaction are physical optimizations introduced only when measured. Protect against decompression bombs and decompression-dependent canonical identity.

Backup/export must capture a **stable pinned root and all its owning objects**, not only a SQLite DB file or segment directory snapshot. Verify export closure and an independent restore. Missing, corrupt or promised objects cause refusal rather than successful backup claims.

## 7. Native remote, publication, and derived data

Separate immutable object transport from mutable **per-ref conditional publication**. A remote may have read caches, shared metadata, and tiered object stores without changing AVC's local source authority. Publication first verifies and retains all new owning dependencies at the target's durability level, then advances the expected ref version; an idempotent receipt makes retries safe. Independent refs need not serialize behind one repository-wide mutex; a multi-ref consistent snapshot, if needed, is an explicitly different transaction capability.

Build derived ancestry/merge-base accelerators, changed-path/path history, blame, copy/rename hints, source search, Git mappings and cache entries independently. Cache results are tied to exact immutable source or revision identities; invalid or missing derived data can be recomputed but cannot authorize a new source root or publication. Restrict raw Merkle membership/absence proofs under directory-level ACLs to avoid leaking names; scoped principals can request server-validated edits without receiving unauthorized sibling hashes.

Serve source reads and conditional edits without checkout. Network APIs are an *adapter around the same library semantics*, not a separate canonical data model. Hosting, CI, review tools, Canto/Ion or agent orchestration, editor CRDTs and deployment remain external to AVC's core.

## 8. Source-code implementation consequences

The repository currently contains **only experimental pure codec/merkle/verifier** code. In particular the old \`merkle::build\` constructs an entire radix directory map from \`FlatMap\`, and \`Verifier\` rebuilds flat maps to check canonicality. These are useful research fixtures, **not** acceptable O(changed-path) production update or admission paths. No durable store, file recovery, native clone, history editing or operational CLI exists.

Replace or refactor the obsolete, unreleased codec in one cohesive implementation migration; preserve only independently useful tests/reference cases. Remove the old \`State\` wrapper and construction-provenance object complexity unless a concrete invariant still needs them. Build persistent conditional editing and incremental proof validation first, then short durable object/root transactions, before an advanced merge algebra or hosted service. Do not carry forward dead adapter code or create shims to maintain a format nobody has shipped.

## 9. Evidence gates, not permanent design paralysis

These are the **only pre-format-freeze research decisions**:

- Qualify the chosen deterministic path-compressed radix leaf encoding/threshold **against one strong ordered chunked alternative**, with measured native updates and validation.
- Qualify a simple typed lossless conflict representation against **flattened expressions** for repeated real rebase/merge/revert without false clean results or unbounded growth.
- Demonstrate that hybrid staged-segment durability, reader leases and independently restorable exports equal the SQLite-only safety baseline. Change physical backend if that fails; do not change canonical IDs.

All other decisions in this document are **selected implementation defaults**, not a request for another whole-system redesign. The current working architecture is sufficiently settled to implement [the first end-to-end native local slice](roadmap.md). When evidence falsifies a material decision, revise the single owner and remove the superseded code/claims. The end-to-end Git sparse-import limitation is a known product boundary, not a reason to delay safe native local work.
