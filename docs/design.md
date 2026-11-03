# AVC native VCS — working architecture

**Revised November 3, 2026. Status: design candidate for the next implementation iteration, not a frozen format or a working source-control product.**

This is the **current owner of intended native architecture**. It supersedes the October/November 2 detailed v0 proposal. The older proposal remains [in Git history](https://github.com/nijaru/avc/blob/1927e1a0862029b0b9bfbbc00e795f6553eab0a4/docs/design.md) at commit `1927e1a0862029b0b9bfbbc00e795f6553eab0a4`; its exact codec assumptions still describe parts of the experimental Rust library and reference fixtures, **not a production format that must be preserved**. Do not silently use the new document to assert that the existing code implements it. All important source must remain in independent Git/other backups until AVC's custody and crash-recovery gates pass.

[Workflows](ux.md) define user-visible behavior. [Roadmap](roadmap.md) defines experiments, qualification, and implementation sequence. [Research](research.md) keeps comparative evidence and rejected alternatives. This is a recommendation intended to survive serious criticism, not a commitment to a specific page size or merge algorithm without data.

## 1. Product and engineering goals

Build a standalone, general-purpose, **local-first native VCS** suitable for regular developers, coding agents, large monorepos, short-lived compute, and decentralized or hosted use. It must be compelling on correctness, recovery, speed, UX, and interoperability, not novelty.

The most important desired capabilities are:

1. **Correct offline source custody.** A complete repository is self-contained. Acknowledged retained source remains reconstructible within its declared durability and retention domain; independent export/backup works without a server.
2. **Fast useful clones.** Obtain the selected tip and current working set first. Fetch older history, other branches, and additional source on demand, without equating missing promised data with absent source.
3. **Safe editable history.** Stable logical change references, exact immutable revisions, rebases, amendments, splits, folds, undo, reviewable candidates, and first-class unresolved conflicts.
4. **Parallel source work.** Independent writable agent workspaces or checkout-free candidates; optional composed views and separate change stacks in one developer workspace. No shared mutable directory is required.
5. **Source APIs without checkout.** Exact reads, diffs, conditional edits, merges, and source queries by immutable ID, usable through a Rust library and later native protocol.
6. **Easy Git adoption.** Clone/import from existing Git remotes, follow upstream, exchange representable revisions and refs, and preserve a truthful mapping. Git must not define native object IDs, source semantics, compression, or protocol.

Not part of AVC's source authority: agent orchestration, process or VM snapshots, editor CRDTs, CI, builds, code review policy, environment/dependency setup, forge UI, or deployment state. Integrations can use AVC's source contracts.

## 2. Identity and canonical source

The preferred logical model is snapshot-first, **subject to measured comparison with changeset-first and causal patch/commutation variants before format freeze**:

- `BlobId`: exact bytes; no line-ending, text-encoding or compression normalization.
- `TreeId`: a typed, deterministic source namespace. Unchanged subtrees share immutable objects.
- `RevisionId`: exact TreeId, ordered parents, author/message/time and selected immutable authored metadata.
- `ChangeId`: a stable logical identity that can follow a change through amend/rebase; separate from the revision version. Divergent successors are not silently unified.

A revision directly names a complete logical source tree; it need not eagerly store an expanded copy of every subtree. Changed-path data, rename/copy hints, blame, path history, graph accelerators and displayed diffs are **derived**, not competing canonical histories. A recorded command may supply move intent, but inferred equality or similarity cannot prove human intent.

Canonical entries should initially include regular file bytes plus executable bit, symlink targets, directories (including empty), and a lossless unresolved-conflict value. Paths are exact raw names subject to safe validation; filesystem normalization, case folding, platform limitations, ACLs and unsupported special files must not silently alter source identity. Portable materialization may be refused even when the source tree remains valid.

Use **typed, domain-separated BLAKE3-256 object IDs** and one canonical binary encoding. Object IDs are hashes of logical uncompressed content, never of a physical compression stream, pack position, remote URL, or local file timestamp. BLAKE3 is a justified current candidate, not a claim that it alone makes clones faster. Authentication, access rights and retention are separate from hash integrity.

### Merkle directory layout is not locked

Preserve persistent structural sharing, deterministic construction, bounded lookup/update costs, efficient range scans and subtree skipping. Compare at least: (a) compact sorted small-directory nodes with hierarchical sharding, (b) path-compressed radix, (c) deterministic chunked B-tree/prolly-style maps.

Evaluate changed-node counts, encoded bytes, lookup I/O, RAM/allocation, edit locality, renames, full diff and cache behavior on real and adversarial directories. Choose thresholds only after **native** benchmarks, not because the old nibble-radix draft used 4 KiB leaves. Once released, canonical layout changes require explicit migration/versioning.

A full decoder and admission path verifies type, canonical bytes, graph acyclicity, resource bounds and the owning closure. A thin client can hold less data without declaring the full closure locally verified.

## 3. Local work and history

Keep **three distinct actions**:

- **Private checkpoint/recovery:** pin observed bytes and their source base, scope, exclusions and retention. This is not a commit or publication.
- **Author:** build an immutable candidate from pinned inputs with exact conditional path/hunk selection. Commit it as a new authored Revision; support an uncommitted first effort.
- **Share/publish:** transfer required objects and conditionally move a named target, with permission checks and exact ref-version preconditions.

Ordinary `status`, `show`, `diff` and `log` are read-only: no hidden commit, capture, network fetch unless requested, hook execution or ref changes. File monitoring/stat caches are optimization hints, not proof of an atomic filesystem snapshot. Capture admits the **exact bytes observed**, with truthful incomplete/changed-during-scan reporting.

Preserve affected current work before AVC itself performs potentially destructive materialization, checkout or navigation. Avoid blind overwrites, especially after an interrupted operation or another editor's write. A durable **operation log** tracks local selection/ref/history transformations and gives scoped undo/recovery; it is not a second source-history DAG or an excuse to retain private source forever.

Make change editing comfortable: amend, rebase, restack, split, fold, squash, and undo operate on immutable candidates. Investigate an optional **absorb** operation: move a later hunk into the earlier editable change that last introduced its lines, previewing the entire restack and refusing ambiguous assignment rather than silently rewriting another change. Preserve revisions as long as they are actually reachable or pinned; evolution relationships can be metadata, not owning retention edges. Editing already shared history requires explicit policy and guarded publication.

### Concurrent changes and workspace composition

Adopt the most useful GitButler idea: **multiple independent or dependent change stacks can be selected and composed into a view**. A stack has an exact base, candidate revisions, dependencies and an unambiguous destination for edits. Explicit path/hunk attribution is supported; automatic assignment may be a helper but cannot silently change ownership.

The composed view is a **derived, reproducible tree**, not a synthetic canonical merge commit and not proof that all branches commute. Conflicting edits, ordered dependencies and mode/deletion changes remain explicit. Selected stacks can be tested together and published separately. A developer may work in a composed directory; concurrent agents may use isolated mutable directories, checkout-free conditional source APIs, or **an explicitly shared composed workspace** when files/runtime are safely compatible. Shared-file and generated-output collisions cannot be made safe by virtual branch ownership alone. Measure the shared-workspace setup savings against collision, stale source and hidden dependency risks before choosing a default. Their work need not use one worktree per commit.

Do not burden everyday navigation with a mandatory named effort, capture, preview, index and workspace ID for every action. Durable identities are required only where they disambiguate independent recoverable work. The detailed user interaction is to be validated in task trials, not inferred from the object graph.

## 4. Clone, checkout, missing objects, and offline guarantees

**Default intent: tip-first and expandable**, not an unconditional full-history clone or an unconditional Git-style depth-one clone.

Separate four independent dimensions:

1. **Revision selection:** selected remote default branch or explicit ref/revision, with a known and authenticated ref version.
2. **History horizon:** minimal current-tip/nearby history by default, deepened on demand; `--history=all` for history completeness. A missing parent is **unknown/promised**, never an empty/root revision.
3. **Object availability:** native object lookups can request missing reachable blobs/trees/revisions through a declared promisor remote; batch/prefetch related work rather than one round trip per object.
4. **Working-copy materialization:** full current tree for manageable ordinary repos, scoped/virtual source for huge repos, or `--no-checkout`. Working-copy scope and historical depth are not the same thing.

`avc clone URL` should give a usable current project with low startup delay and disclose how much source/history is locally retained. A native server should permit a metadata-first checkout-free clone and fetch only the paths an agent actually uses. For ordinary editor/CLI tools which read local files, either materialize their requested scope or use an optional supported virtual filesystem; do not pretend an absent file is available.

The client must distinguish: present+verified, promised-but-not-present, unavailable promisor, inaccessible/unauthorized, truly absent, and corrupt object. No authoritative commit, backup, offline-complete claim, or GC step may depend on a missing promised object. A thin client may submit conditional edits and let an authorized full holder complete verification/admission. Offline operations work on retained scope; `avc hydrate --all` (illustrative) deliberately obtains and verifies a full requested closure before making an offline completeness claim.

Native promises must include adequate remote provenance and closure/ref requirements to avoid trusting a server-supplied hash alone. Need-to-know access controls must not leak path names through raw tree proofs.

**Git remote adaptation:** use Git partial clone/filtering where servers support it and report limitations. Do not blindly force Git `--depth=1` for ongoing developer work: shallow boundaries can penalize later fetches and merge-base queries. The AVC native protocol has no reason to inherit those Git negotiation costs.

## 5. Merge and conflicts

Use deterministic three-way **source value** merging with explicit merge bases and a versioned text driver. Equality/base rules, recursive directory merges, whole-subtree deletion-vs-modification protection, executable/symlink/binary treatment and no-loss cases must be correct before optimizing clean-merge rates.

**Reopen the old all-optimal-LCS text merger.** Its public-history reference was conservative but incurred extra conflicts relative to Git/Jujutsu; exact confluence alone did not establish better user outcomes. Benchmark a practical modern line merge with alternative diff algorithms against clean merges, unnecessary conflicts, false-clean outputs and cost.

Represent unresolved alternatives as typed immutable values so repeated rebase/merge can operate without treating marker text as the source authority. **Reopen the nested-conflict encoding** against normalized/flattened merge expressions (including Jujutsu-like signed expressions), testing cancellation, repeated rebase/revert, binary entries, path/type conflicts, provenance and resource bounds. Select the algebra only when losslessness and ordinary editor presentation are demonstrated. Runtime timeout, out-of-memory, unavailable promised input, or I/O failure must **abort/preserve inputs**, never fabricate a semantic conflict.

No automatic merge guarantees program correctness. Tests and review evidence attach to an exact candidate and execution scope; they do not become source identity or retention authority.

## 6. Physical storage and durability

Logical source identities and immutable objects do not mandate a particular database or physical pack format.

**Leading physical candidate:** SQLite for atomic local roots, pins, receipts, ref versions and indexes; staged, immutable payload segments for large or numerous objects; optional small-object coalescing. Compare to the current SQLite-only prototype on equal crash/recovery and performance gates before selecting. The local engine must run without a daemon, network or filesystem mount.

**Leading codec:** Zstandard for payload/segment compression, benchmarked against zlib and other relevant codecs *after* accounting for Git's cross-object delta compression. Explore dictionaries for tiny objects, optional inter-object deltas and bounded large-object chunking. Chunking, compression level, segment grouping, caches and compaction should be physical policy, **not revisions of logical object IDs**. Choose measured default trade-offs for first checkout, random read, CPU, RAM, disk and transfer; do not embed speculative numbers in the wire format.

For every acknowledged retained root: dependencies have been hashed, validated and made durable; a short transactional root change follows; crash recovery can distinguish committed and staged work. Compaction and GC never delete object closure required by any protected root, active reader, export or promised durable receipt. Backups/export have independent verification and a restore probe. The ordinary filesystem is uncontrolled, so materialization needs path- and version-guarded updates and before-image protection.

## 7. Native synchronization and hosted scale

A **native object protocol** offers exact objects, bounded batches, resumable transfer and explicit missing-object resolution. Clients and caches may reuse immutable objects by ID. Fetch planning and prefetch should match a client's scope, not force all history or full checkout. Larger transfer bundles and reuse/caching should be measured against per-object access, not presumed.

Remote refs use **per-target compare-and-swap** and guarantee that all objects required by an acknowledged published target are available under the target's retention policy. A set of unrelated refs need not share a global write lock; requests that truly need a multi-ref atomic view must explicitly use a separate transaction/snapshot mechanism. Replication, caching and consistency are specified honestly.

Pando suggests independent ref coordination, reusable transfer artifacts and LSM-like compaction. Scion suggests tiny clients, RPCs for source queries, derived indexes and tiered caches. Neither prescribes Git packfiles, a centralized server, or server-side commits for AVC. Optional hosted source APIs and bulk index services can scale later without changing the offline core.

## 8. Git adoption without Git authority

Git interoperability is **an early product capability**, not merely a final migration tool, but it is a boundary adapter.

- Clone/import a chosen Git branch into native identity, preserving exact representable bytes, tree structure, commit ancestry and available metadata with explicit foreign-ID mapping.
- Retain original Git object IDs and refs as **foreign identities**, not new AVC content IDs; unchanged imported Git objects may be reused for round-trip export where valid.
- Fetch upstream changes incrementally, update remote tracking refs, and compare or merge against native work. Repeated sync must not duplicate history or silently choose rewritten successors.
- Export/push representable native revisions to GitHub/GitLab/ordinary Git, with explicit Git commit mapping and guarded remote updates.
- Report nonrepresentable features (typed unresolved conflicts, empty directories, unusual names or metadata, unsupported file modes, signatures, LFS behavior, shallow boundaries) before claiming a lossless round trip. A new Git commit changes Git IDs and invalidates some signatures; never promise universal identity preservation.
- Git CLI coexistence may be useful, but **do not create a dual writable Git/AVC authority by default**. Evaluate a colocation/mirror mode separately, including external Git mutations, GC, ref races and checkout ownership.

Use an established Git implementation or subprocess adapter at the boundary before attempting a second fully native Git pack/protocol stack. Avoid making the native engine a disguised wrapper.

## 9. Derived data, safety and extension boundaries

Compute and cache commit generation/ancestry accelerators, path history, blame, copy/rename hints, changed-path Bloom filters, structural diffs, and source-search indexes as **derived data**. Missing or stale indexes must degrade performance, not correctness. Derived data can be eagerly or lazily computed, locally or remotely.

No checkout, merge, clone, source query, export or import automatically executes source-provided hooks, code, build steps, generators or agent tools. Authenticate remotes, bound untrusted input and decompression, validate path containment and symlink behavior, preserve privacy/authorization, and handle malicious or incomplete closure correctly.

Protocol and stored objects must be versioned explicitly. If an experimental format changes, replace old prototype fixtures with independently verified new vectors; never claim backward compatibility to a never-released draft.

## 10. What must still be decided by evidence

These are **open engineering selections**, not parallel production implementations:

- Snapshot-root authority versus incremental changeset authority **and causally commuting changes** on measured large-agent workloads (working recommendation: snapshot).
- Persistent directory tree layout and canonical node bounds (no selected size/shape).
- Normalized conflict algebra and practical text merge driver (no new frozen driver).
- SQLite-only versus SQLite + immutable segments under equal durability requirements (hybrid is leading candidate).
- Native tip-first clone protocol, missing-object promises, safe Git partial-import adaptation and virtual filesystem limits.
- Multi-change composition UX and the earliest slice justified by user task trials.

**Decision rule:** preserve the correct invariants and essential workflows; change any mechanism when controlled performance, usability or correctness evidence supports a better solution. Do not reselect merely because a newer platform is fashionable, and do not preserve an old mechanism merely because earlier prototypes implemented it.

The next actionable implementation and qualification sequence is [docs/roadmap.md](roadmap.md).