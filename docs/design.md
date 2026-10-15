# AVC — native version control system design

**Architecture decision: 2026-10-15.** **Status: selected v0 design, not implemented or qualified.**  
[Research and rejected alternatives](research.md) explain why. [Verification and delivery plan](roadmap.md) defines what must be proved. Older incompatible proposals are superseded, not additional requirements.

This is a deliberate design decision, not a universal proof of optimality. Reopen a foundational choice only for a concrete correctness counterexample or convincing *complete-workflow* evidence. Do not reopen it solely because an incumbent uses a different model.

## 1. What AVC is for

A **general-purpose, native, local-first VCS** usable by one person editing ordinary files, teams collaborating asynchronously, and multiple agents in permanent or short-lived environments. Relevant environments include desktop/laptop, SSH/VPS, cloud workspace, container, microVM, CI/build, and sparse source-query service. Source, workspace, recovery, validation and deployed runtime are **distinct resource types**.

The primary job is to preserve exact source and support safe transitions between versions. It must also:

- Keep exploratory edits recoverable without converting every save into an immortal published revision.
- Give every independent writer its own mutable files, while sharing immutable source and permitted caches.
- Answer historical source/diff/merge questions by ID **without a checkout**.
- Allow sparse/thin remote clients *without requiring* a hosted service for a complete offline repository.
- Preserve ambiguous conflicts instead of manufacturing a clean result; test the exact combined source before protected acceptance.
- Protect acknowledged objects, captured work and draft handoffs across the claimed failure model.

**Nonrequirements:** Git interoperability, Git object or pack formats, Git refs, agent orchestration, LLM transcripts, mandatory daemon/FUSE, process/VM snapshots, CI/deployment orchestration, and universal causal patch commutation. Native portability and a self-contained backup/export are required even without Git.

## 2. Authoritative model — two immutable identities

**Decision: canonical exact source snapshots.** One immutable State commits to a complete typed logical source tree. One immutable Revision commits to that State plus history and authorship. There is *one* source authority: content-addressed source objects. Edit recipes, path deltas, blame and history indexes are optional derived/verified data, not competing byte truth.

Conceptual schema (semantic contract, not implementation code):

    Entry =
       Directory { child_map: MapId }
     | File { content: BlobId, executable: bool }
     | Symlink { target_bytes: ByteString }
     | Conflict { value: ConflictValueId }

    State { namespace: MapId } -> StateId

    Revision {
      state: StateId,
      parents: ordered Vec<RevisionId>,
      logical_change: ChangeId,
      author, description, authored_metadata,
      provenance: optional ProvenanceId
    } -> RevisionId

    Evolution {
      prior_revisions, successor_revisions, kind
    }

**StateId** is the hash of typed source tree content, including empty directories and unresolved source values. Two histories producing exactly the same logical source yield the same StateId in the same canonical format. **RevisionId** identifies the immutable authored/history record and can differ for an equal StateId. **ChangeId** is a locally generated 128-bit random logical identity that normally persists across amendments/rebases; it is not a patch's physical identity and has no commutation guarantees. Divergent successors and split/fold evolution are explicitly represented. No timestamp selects a winner.

Parents represent historical ancestry, *never instructions for reconstructing bytes*. A merge with two or more parents directly identifies its complete resulting State. Changing a description or author metadata creates a new immutable RevisionId; this is intentional. User notes, reviews and attestations can be referenced separately when they should not rewrite history, but there is no second mutable source description.

**Objects and IDs.** v0 native ID algorithm: **BLAKE3-256** over a domain-separated, versioned, length-framed canonical binary encoding for each object kind (blob, map node, state, revision, conflict value, provenance). Map order, integer encoding, duplicate handling, unsupported types and bounds are specified, not inferred from Rust serialization. The digest identifies logical **file bytes**, independent of whether content is physically compressed, chunked or moved. Short IDs are display-only. SHA-256 can be supported as an explicitly new format/compliance profile if required later; no Git SHA restrictions.

Canonical formats are genuine long-term commitments: if a different tree-node encoding changes map hashes, StateIds also change. Keep versioned decoders and explicit old↔new migration mapping; **do not** claim that canonical tree layout is a free-to-change derived cache.

## 3. Native source namespace and its data structure

**Choose one canonical structure:** a hierarchical immutable Merkle directory tree, whose per-directory child map is a **deterministic path-compressed radix tree with small sorted inline leaves**.

- Entries are keyed by exact raw byte name components, sorted in byte order. No silent Unicode normalization, case folding or OS-specific path conversion.
- A small map remains one sorted encoded leaf. Once above a fixed **4 KiB encoded leaf target** (subject to a conservative hard node limit), partition deterministically by key nibbles, compress common prefixes and inline small terminal maps. Node layout depends only on live key/value pairs and format version, **not insertion order**. Path compression avoids traversing one full node per byte.
- An update copies only changed content and the immutable map nodes along affected paths. Equal subtrees have equal hashes; diff can skip them. Reusing a directory subtree under another name does not enumerate descendants.
- Explicit empty directories are supported. File content is opaque; no UTF-8, AST, language parser or line-ending conversion is required.

This is a deliberate choice over a flat directory array at unlimited width (large rewrite cost), insertion-order-dependent mutable B+ pages (unequal roots for equal maps), and prolly/POS pages (good ordered sharing but nonlocal page-boundary shifts and a more involved canonical merge). Prolly is a **performance alternative**, not an architectural undecided fork. The specific 4 KiB threshold and nibble implementation are tuning details inside the chosen radix design; before publishing a stable format, record canonical vectors and pathological-key tests.

**Native source metadata policy:** v0 tracks regular-file bytes plus executable flag, symbolic-link target bytes and empty directories. Preserve all admitted raw names. Disallow empty, "."/"..", NUL and slash components; bound component/path lengths explicitly. Reject unsupported platform names, name collisions, devices, arbitrary ACL/xattr or hard-link *identity* requests rather than silently converting them. Do not store timestamps/owners as canonical source truth; they are filesystem observations. No silent content filters or generated-content execution.

**Subtree operations are source construction**, not canonical patch history:

    Put(path, expected_value, new_value)
    Remove(path, expected_value)
    CopySubtree(source: SubtreeId, destination, expected_destination)
    MoveSubtree(from, to, expected_from, expected_to)

Copy and move relink immutable subtree values. Copying by *content SubtreeId* avoids retaining an unrelated historical revision as an implicit dependency. Preconditions include a verified **absence/path witness**, not only a client assertion. Validate all operations against the pinned immutable base before atomically constructing the result. For a move-and-edit sequence, define structural relocation first, then explicitly checked destination file edits. Reject overlapping/cyclic/conflicting namespace instructions. Ordinary before/after filesystem scans may fail to identify an intentional rename; the correct byte state remains recordable without invented move history.

## 4. Source construction, workspaces and recovery

The core construction API is:

    apply(base: StateId, conditional_edits) -> Result<StateId, TypedError>

A complete local client may build the new root itself. A sparse client can present changed bytes and authenticated Merkle path/absence witnesses, or ask an authorized service to verify and construct a root. If neither has the necessary source closure, store a **pending edit/draft**, not a falsely authenticated complete StateId.

**One workspace is one writable directory**, not an actor credential or process sandbox. Its durable mutable resources are separate:

- **Selection:** currently chosen RevisionId + selection generation.
- **Capture head:** most recent acknowledged recovery CaptureId + separate generation.
- **Materialization:** last intended source, actual destination, progress/outcome, and managed writer/ownership epoch where applicable.

No single repository-wide logical operation head moves on every watcher event. An actor may have many sessions and workspaces; a container may terminate while its immutable submitted work survives. All ordinary source operations against an immutable base may prepare in parallel, with only short root-selection transactions serialized as needed.

**A workspace's actual files are external mutable input.** Status is a fresh, fallible observation; never persist "clean" as a permanent guarantee. Watcher/inode/mtime data are hints and overflow triggers recrawl. A scan without writer exclusion may capture files that never coexisted in that combination. Record its consistency class and coverage, rather than promising a point-in-time image. Invalid, missing and ignored paths are visible, not quietly treated as empty. Unsaved editor buffers are beyond AVC's observation.

**Three retention classes:**

1. **Recovery capture:** bounded private record of admitted exact bytes/deletions, linked to pinned base with coverage, observed consistency, durability and expiry. No semantic authored revision, remote upload, public CI or review event implied. An optional agent/editor hook can capture frequently; no daemon is required. Quota exhaustion causes an explicit failed *new* capture, not silent early deletion of already acknowledged captures.
2. **Authored revision or protected draft:** immutable full source State and history; retain by explicit named/workspace root or a documented orphan/draft grace policy after rewrite/closure.
3. **Accepted target history:** protected by shared roots and backup policy. Never reclaim acknowledged accepted source because no editor currently has it open.

Default proposed local retention policy: **7 days for unpinned private captures, 90 days for orphaned authored revisions, unlimited while explicitly pinned or accepted**, with repository-visible quota/retention settings. A confirmed capture promises only its stated coverage and horizon, not off-device backup.

**Safe restoration.** Default guaranteed non-destructive restore/switch materializes into an **exclusively created new directory**, never clobbering the existing editor directory. A root transaction and an OS directory update are not one atomic operation. In-place updates require genuine writer exclusion or offer explicitly weaker semantics with preserved before-images and pending-intent recovery. Check-then-rename, advisory locks or database epochs alone cannot revoke external file descriptors. A verified materialization is separately recorded; interrupted materialization remains incomplete.

## 5. History editing, merging and conflicts

A merge is an explicitly identified transformation of exact immutable source States and ancestry, not a union of logical ChangeIds. For base B, left L, right R:

    L == R                -> L
    L == B                -> R
    R == B                -> L
    both directory values -> recursive child merge
    supported content     -> pinned deterministic text/structure driver
    otherwise             -> typed unresolved source value

Include symlinks, executable mode, file-vs-directory, delete-vs-edit, binary replacement, rename/edit and empty directory cases. No automatic merge implies program correctness. For multiple merge bases use a defined recursive/virtual-base procedure with conflict-preserving inputs; never pick one arbitrary ancestor. Higher-order/n-way composition has an **explicit input ordering and recipe**, not a false promise of associativity. Complex conflict expressions are represented as bounded immutable DAG values rather than recursively expanding an unbounded marker string; on complexity limits preserve an unresolved conflict instead of erasing alternatives.

**Conflict source and conflict origin must be separate.**

    ConflictValue { base_terms, distinct alternative entry/subtree terms }
    ConflictOrigin { input_revision_ids, path, roles, driver, recipe }

The first identifies the exact unresolved *source expression* including absence; deterministic encoding canonicalizes value-equivalent alternatives while preserving distinguishable bases/terms. The second carries the **input** revision identities, historical roles and algorithm provenance. The output Revision references this provenance object; the provenance object must never refer back to its owning output RevisionId, which would create a hash-reference cycle. An API obtains the owner from the referencing Revision. Identical unresolved source with different origins has equal StateId; its RevisionIds/provenance can differ. Do not indiscriminately flatten nested conflicts from old merges. Rendering textual markers is a **lossy workspace view**, not source bytes; an unchanged marker presentation round-trips using an explicit manifest. Binary/directory conflicts may never have marker text. Any resolution is a new State/Revision, preserving inspectable prior terms and provenance. Reuse resolutions only against verified identical conflict/context inputs.

**History operations:**

- **Amend:** new State and Revision, same logical ChangeId, predecessor evolution recorded. Never mutate an old revision.
- **Rebase single-parent C from P onto Q:** compute 3-way merge of State(P), State(Q), State(C), create new Revision with parent Q and C's logical ChangeId; preserve C and record evolution. Rebase multiple selected changes topologically. Multi-parent rebasing must use an explicit recipe, not pretend to be linear.
- **Undo:** append a resource-root transaction restoring previous private selection/pins with current-generation preconditions. Does not implicitly overwrite the physical working directory.
- **Revert:** generate a new inverse-*content* revision against current source, potentially with conflicts. It cannot promise causal removal of just one origin of duplicated content.
- **Reapply:** explicitly apply a previously reverted effect to a new base; neither stable ChangeId nor previous ancestry bans legitimate reapplication.

This intentionally adopts **snapshot-state semantics with first-class symbolic conflicts**, *not* Darcs/Pijul change commutation. Those systems supply real alternate algebra, but most ordinary file writers do not expose the causal operation stream their strongest properties require. If later use cases demand collaborative character-level causal editing, provide it as an editor/merge input; do not silently redefine the whole VCS.

**Default merger:** a conservative, deterministic rename-aware three-way tree/text driver with conflict preservation. Optional token/semistructured drivers can be enabled per file type *only with measured false-conflict versus incorrect-automatic-merge evidence*. An LLM may propose edits/resolutions through the same validated source API; no model call is part of canonical source decoding, replay or storage.

## 6. Durable local storage, object lifetime and repair

**Select a concrete local reference backend:** one SQLite transaction authority for mutable roots, operation outcomes, request epochs, retention pins and **small canonical objects**. Immutable large blobs are stored separately as content-addressed files/segments with an explicit two-phase admission protocol; no custom LSM/Raft is required locally.

1. Authenticate/validate request and pin an object-staging transaction or write-intent epoch so the collector cannot reclaim newly prepared data.
2. Stream/hash canonical content, write temp immutable files, flush file and parent-directory metadata as the supported filesystem requires; verify object digests and canonical dependency closure.
3. In one short durable SQLite transaction, check expected resource generations and request identity, publish referenced objects and root/pin changes, record before/after operation/outbox/acknowledgement outcome. A duplicate retry queries the durable original result.
4. On crash, committed roots remain complete; uncommitted staging may be reconciled or reclaimed *after* reader/writer epochs and an expiry condition. Acknowledged revisions never reference unprotected missing objects.
5. GC marks all accepted roots, explicit pins, unexpired recovery and orphan grace roots, live readers and in-flight staging. Retire old storage only after reader safety. Backup/restore includes authoritative mutable roots **and the full referenced immutable closure**; lost hashes alone are not repair.

SQLite reference uses WAL and a *durability mode appropriate to acknowledged writes* (e.g. synchronous=FULL with declared filesystem conditions). A local crash test cannot prove power-loss behavior. No global repository "latest operation" CAS is needed for unrelated workspace capture/selection. Multiple short metadata transactions may physically serialize, which is different from false **logical** conflicts.

**Physical optimization policy:** small objects may be packed/compacted by size or access locality, with Bloom/index metadata and block checksums if justified. Ordinary files are stored as whole logical byte blobs; optional physical chunking and bounded delta chains for very large/frequently edited assets never change their BlobId. There is no canonical globally sequential object ID and no requirement to use Enroute layers, Git packs, consensus or S3. A hosted store can use immutable object storage plus its own transactional target authority behind the *same native object semantics*.

## 7. Source queries, native exchange and deployment

The same versioned source-operation semantics serve a local full repo or remote service:

- Get object/file byte range or typed path; list tree; diff States; inspect history/ancestry; inspect conflict alternatives; create immutable State from verified conditional edits; create/rewrite Revision; propose/compose/validate/accept.
- **Full local client:** holds all objects needed for its selected retained roots and works fully offline.
- **Partial client:** knows which content is promised but absent. Can hydrate by requested object/tree path or save pending conditional edit bundles; it cannot falsely report an absent object as an absent path.
- **Short-lived cloud/CI agent:** use sparse exact-object requests or a compressed batched subtree/history transfer, with immutable shared read caches. No full-history checkout required for metadata-only work.
- **Ephemeral handoff:** the service durably acknowledges exact submitted source/Revision and a protected draft/root **before** the worker VM terminates. Uploading unpinned blobs alone is not durable work submission.
- **Production:** deploy a built artifact (OCI image/binary/package) with source RevisionId and validation/build provenance; a VCS daemon or full source checkout is not ordinarily required.

Use both **batched sparse object queries** and **compressed/resumable bulk bundles**. Choose based on client cache/network and requested closure. Hashes give integrity/equality, not authorization or cross-tenant access. Remote source APIs require authenticated permissions, including any path/directory-scoped disclosure. Cross-tenant caches must not reveal object existence or bytes without authorization. Fetched repository configs do not automatically execute hooks, tools, build commands or merge drivers.

Native exchange is not a Git compatibility bridge. There must also be a **self-contained inspectable native export** of tracked source, history/pins and verifiable hashes so developers can keep independent custody of their work.

## 8. Acceptance: select → compose → validate → publish

A candidate binds (a) target name and observed generation/state, (b) exact selected RevisionIds, (c) the deterministic merge recipe and result StateId/RevisionId, and (d) a versioned validation policy. Tests/approvals are **separate evidence referencing that immutable candidate**, including exact toolchain/test recipe/input assumptions. Evidence cannot be part of the candidate's own hash.

Validation runs outside the short target transaction. The authority then atomically rechecks target generation, authorization/policy epoch and accepted evidence, advances only that target, and records a receipt/outbox event. If the base changed, recomposition and revalidation are required as policy dictates. Build/CI caches can reuse results only when their complete declared inputs agree—not merely because touched file paths don't overlap.

**Default** for protected targets: validate before acceptance. A project can deliberately choose separate *accepted* and *verified* targets with post-accept/pre-deploy checks; these are visibly different guarantees. Batching multiple independent proposals can reduce the number of target updates, but a single linear target always has an essential ordering point. A tool cannot automatically guarantee that a syntactically clean merge is a correct program.

### End-to-end example: three agents, one accepted source

A developer selects Revision R0 with source State S0, edits ordinary files, and captures recoverable bytes C1. C1 advances only the private capture head; it does not change R0 or a shared target.

Two independent remote workers receive S0 by a sparse cache or native bundle. They produce immutable (R1,S1) and (R2,S2), each preserving its source bytes and independent history. Their draft roots are durably pinned on the service *before* their containers terminate. Neither worker must rewrite R0 or advance main.

A reviewer composes R1 and R2 against S0. The merger creates a new exact State S3, possibly containing typed conflicts. The chosen resolution, if any, creates State S4 and a new integration Revision R3 with ancestry [R1,R2]; the input revisions remain immutable. Candidate K binds R3/S4, the merge recipe, policy and the target's expected generation.

Tests execute against **exact S4**, not mutable worker directories. A protected target advances to R3 only if evidence and target/policy preconditions still hold. If another accepted revision arrives first, K remains inspectable but cannot silently land; the integration must be recomputed and revalidated under policy. The developer's earlier Capture C1 and both original authored revisions are not lost.

This workflow uses the same logical source identities on a laptop, VPS, CI worker, remote agent and native server. A production build may package S4 into an artifact; the running service need not contain a VCS repository.

## 9. Format boundaries, security and failure model

All canonical objects are immutable and validated at admission/untrusted read boundaries. Bounds apply to paths, history depth, source-tree recursion, multiway conflicts, object sizes, expansion ratios, request deadlines and staged-data retention. Materialization uses path-safe directory-relative operations and never follows malicious symlink chains outside destination. Deny unsupported name/case collisions rather than rewriting tracked files.

Content-addressing does not guarantee availability, authentication, confidentiality or semantic correctness. Signed provenance and review attestations are optional separately authenticated records. Encryption and access-control policies belong to storage/transport boundaries with clear key and revocation semantics. A secret already admitted into history/captures/backups is not deleted by adding an ignore rule; redaction requires explicit retained-root and backup handling.

**Failure guarantees are scoped and observable:** a locally acknowledged transaction survives the stated SQLite/filesystem crash domain; remote durable submission survives the service's documented replica/storage failure domain; a capture protects only its reported bytes/coverage/horizon; no uncoordinated directory scan is an atomic multi-file image; no in-place checkout can guarantee protection against arbitrary external writers.

## 10. Final decision register

| Question | Final v0 design decision | Why |
| --- | --- | --- |
| Canonical history | **Immutable snapshot-root State + Revision DAG** | Exact source identity for arbitrary writers, direct historical read, merge, safe copy/backup; persistent trees share bytes |
| Logical changes | Stable ChangeId + immutable revision evolution | Rewriting without conflating source identity with authored intent |
| Native source tree | **Deterministic bounded compressed radix per directory**, small inline leaves | Equal-value canonical roots, bounded wide-directory updates, subtree reuse, sparse lookup |
| Filesystem source | Typed regular/executable files, symlinks, explicit empty dirs, unresolved conflicts | Practical opaque-source semantics without inheriting Git format |
| Conflict authority | Symbolic typed source terms; origins in separate immutable provenance | Exact unresolved state equality, preservation of multiple histories |
| Merge/rebase/undo | Conservative 3-way plus typed conflicts; new revisions for rebase; root undo distinct from content revert | Explicit, explainable behavior without unproven patch algebra |
| Recovery | Separately pinned, bounded capture overlays; independent workspace selection | Cheap intermediate progress without polluting authored history |
| Workspace | Independent writer directories, generation-scoped resource roots, honest observation | Avoid spurious concurrency conflicts and false filesystem atomicity |
| Durability | SQLite transactional authority + protected immutable large objects | Proven primitive, one short root authority and explicit blob/GC protocol |
| Sync/API | Full local + sparse/thin remote, batch and exact-ID retrieval | Local independence *and* efficient ephemeral/cloud work |
| Acceptance | Exact candidate validation, per-target CAS, explicit accepted/verified policies | Correct ordering and provenance without global draft bottleneck |
| Git | **No requirement**, later adapter only if justified | Freedom to design genuine native semantics |

**Deliberately left for engineering calibration, not another architectural competition:** exact leaf/max-node tuning, compression/chunk thresholds, parser-specific merge drivers, retention defaults per deployment, optional physical packing/cache policies, and platform qualification details. These do not alter the chosen State/Revision/Workspace/Publication model. If a **material counterexample** overturns a decision before v1, revise it and document why; do not maintain simultaneous contradictory canonical designs.

**Implementation status:** none of this native VCS is implemented. Existing Rust 0.0.1 remains a Git-command wrapper with separate JSON log and must not be represented as trusted native storage.
