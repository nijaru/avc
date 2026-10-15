# AVC architecture: source state first

**Re-evaluated:** 2026-10-14  
**Status:** concrete native *reference design*, not implemented, benchmarked, or format-frozen  
**Evidence:** [research](research.md) · **Delivery gates:** [roadmap](roadmap.md)

This document supersedes the previous assumption that parent-relative changesets should be canonical. The goal is to make the best general-purpose source-control system for ordinary development **and** rapidly growing agent workflows. Git, Jujutsu, Scion, Mononoke, Pijul and Enroute are sources of evidence, not ancestors whose formats or algorithms must be reproduced.

## 1. Derive the core from user-visible contracts

A VCS is first responsible for preserving, identifying, retrieving and combining **exact source states**. The work is done by people, agents, scripts and editors, not necessarily through a controlled edit API. It can include malformed text, binaries, directory moves, symlinks and files produced by arbitrary processes.

Representative environments:

| Location | Meaningful operation | Binding requirement |
| --- | --- | --- |
| Laptop, desktop, persistent VPS | Normal editor, offline source history and recovery | Source, history and undo can work without network or daemon |
| SSH/cloud developer environment | Move/reopen work on another machine | Durable independent state; separate filesystem materialization |
| Short-lived container or microVM | Inspect, edit, test, submit, destroy | Share immutable source/cache; acknowledge handoff before destruction |
| CI/build job | Read exact source and produce a verifiable artifact | No unnecessary history clone; isolated exact materialization if needed |
| Source/query service | History/blame/diff without working copy | Immutable query API over exact IDs |
| Large team/agent fleet | Many private attempts, few deliberately accepted results | Cheap independent writes; narrow shared-target coordination |
| Production runtime | Deploy a pinned artifact or source tree | Link provenance; do not require running the VCS server |

Separate **source**, **mutable workspace**, **private recovery**, **shared publication**, and **execution/deployment artifacts**. Do not put processes, prompts, model internals, build caches or VM filesystem images into canonical source history.

**Native is a product decision.** Git format, object hashes, on-disk files, refs, pack format, interoperability and even the word "commit" are **not requirements**. A future adapter is allowed only on its own merits. The initial system must offer a self-contained, inspectable native export and backup before becoming trusted.

## 2. Canonical authority: source values, revisions, and provenance

**Reference decision:** every immutable revision commits to the exact resulting **StateId** (persistent content-addressed source namespace) and ancestry. Changed-file summaries and edit recipes may accelerate construction/transfer, but they do not compete with the source root as an authority. Canonical source means *tracked source values*, not mutable bytes in an editor's directory.

### Minimal conceptual types

Illustrative contracts, not finalized serialization:

    State = persistent typed namespace root

    Entry = Directory { children }
          | File { content_id, executable, kind }
          | Symlink { target_bytes }
          | Conflict { conflict_value_id }

    ConflictValue = {
        kind,
        base: Optional<EntryValue>,
        alternatives: canonical collection<Optional<EntryValue>>
    } // NO revision IDs, author, wall clock, or merge-driver identity

    Revision = {
        format_version, state: StateId,
        parents: Vec<RevisionId>,      // ancestry, not content derivation
        logical_change_id: ChangeId,
        author, description, immutable_metadata,
        provenance_bundle_id: Option<ProvenanceBundleId>
    }

    ProvenanceBundle = { conflicts: Vec<ConflictOrigin>, merge_recipe: Optional<RecipeId> }
    ConflictOrigin = {
        path, conflict_value_id,
        source_revisions, merge_recipe_id, side_attribution
    }
    Evolution = { prior_revision_ids, successor_revision_ids, operation }

    WorkspaceSelection = { workspace_id, generation, selected_revision }
    WorkspaceCapture = { workspace_id, generation, latest_capture_id }
    Materialization = { workspace_id, operation_id, source, destination, stage }
    Target = { name, generation, revision, policy_epoch }
    Operation = { request_id, affected_resources, before, after, outcome }

The three mutable workspace records are **logically independent resources**. A high-frequency watcher advancing the capture head must not continuously invalidate an unrelated selection transaction. One physical local database may still serialize short writes.

A **StateId** identifies source values, including a typed *unresolved expression* when present. Equal resolved files, modes, directory names and unresolved expressions yield equal IDs within one canonical format irrespective of the authors or commits involved. If two conflicts have the same alternative values but different origins, they share a ConflictValueId; distinct bases or alternatives may give distinct IDs because their *unresolved semantic expressions* differ. Identical fully resolved source produces the same StateId even if authorship and ancestry differ.

An **EntryValue** may be an absent path, ordinary file, directory subtree or symlink; conflict terms can therefore express delete/edit and directory collisions, not just lines of text. The canonical encoding must define term ordering/deduplication, handling of already-conflicted inputs and bounded expansion. Multiway conflict normalization is a **format-blocking specification and test**, not something solved by sorting two diff hunks. Preserve all distinguishable alternatives until a safe resolution is defined.

A **ConflictOrigin** binds a specific conflict value *occurrence* in a revision to the revisions and algorithms that produced it. The owning Revision references an immutable **ProvenanceBundle** of such occurrences. The bundle references only **input** revision IDs, content terms, paths and recipe IDs; it cannot refer back to its owning output RevisionId (that would create a hash cycle). The bundle is reachable from the Revision but **does not influence StateId**. The same source conflict can therefore have different origins and different RevisionIds without changing source equality. The API may return a conflict value with zero or more separately loaded origin records. A resolution records the exact conflicted StateId, path/value, chosen result and evidence, without silently mutating an existing source tree.

A **RevisionId** identifies an immutable history record (StateId, ordered parents, logical change identity and metadata), not the complete on-disk working directory. LogicalChangeId labels related versions, **not** commutative patch identity, and an amendment/rebase makes a *new* RevisionId. Concurrent successors may diverge. Source equality, revision identity and logical change identity are different tests. Rewrite/evolution records track split/fold and predecessor/successor history.

**Metadata trade-off:** immutable author/description are part of the reference RevisionId. Therefore changing a description may require rewriting ancestry-linked descendants, as it can in existing commit DAGs. A separate versioned note/attestation system could avoid this but would introduce concurrent annotation selection, signature and sync semantics. **Do not add it merely for novelty.** Test whether description-edit churn is material for real users before choosing separate canonical identity and annotations.

**Source namespace:** components are exact byte strings with published validation rules. Disallow empty, dot, dot-dot, slash and NUL components; do not silently normalize Unicode or fold case. Support regular/executable/opaque file content, symlink target bytes, explicit empty directories, and typed unresolved entries. Unsupported device nodes, hard links or platform-specific metadata must fail explicitly if users ask to preserve them; cross-platform materialization must reject collisions rather than alter names.

**Encoding consequence:** a canonical Merkle tree's page/node encoding participates in StateId. A different canonical encoding may change IDs, even for identical logical files. Publish deterministic hash/encoding vectors before stability, preserve decoders during migration and map old IDs explicitly. Physical compression and object placement need not affect IDs, but canonical tree layout does.

## 3. Constructing source without confusing it with the filesystem

AVC must support **ordinary external file writers** and **headless exact-ID operations** through the same source semantics. An immutable State is a deliberately constructed collection of bytes and entries. It is *not* a claim that those files all existed simultaneously in an uncontrolled working directory.

### Source API

    apply_edits(
      base: StateId,
      edits: Vec<ConditionalEdit>,
      observed_inputs: Optional<ReadWitnesses>
    ) -> Result<StateId, SourceConflict>

    Put(path, new_entry, expected_old_entry_or_absence)
    Remove(path, expected_old_entry_or_subtree)
    CopySubtree(source_subtree_id, destination, expected_destination)
    MoveSubtree(source_path, destination, expected_source, expected_destination)

The entire edit set is checked against a pinned immutable base. If expected entries are stale or conflicting namespace operations overlap, reject the **whole** operation, not just its last path. An absent-path precondition needs a verifiable parent/path witness; a bare client claim that a file was missing is not sufficient.

Changing a few files builds only new file objects and affected persistent tree nodes, reusing unchanged directory subtrees. An unchanged subtree move is a namespace-binding update. Copying a subtree by **SubtreeId** does not require retaining the historical revision in which it first appeared; the copied subtree's *content closure* must remain reachable.

The transformation API is an efficient way to **construct** a State. Its list of edits is *not* a required permanent causal edit log, and it may be discarded after the resulting State and any selected audit/provenance have been recorded.

### Authorship from an ordinary working directory

The file observer hashes and admits actual byte sequences, checks tracked/admitted path coverage and records failures. It may select stable per-file observations and construct a valid immutable State whose bytes can be reproduced, but **a scan without writer exclusion cannot prove those files coexisted at one instant**. Do not represent a quick double-stat as an atomic filesystem snapshot.

A user may intentionally finish a revision from a complete, clearly labeled best-effort observation; the default full-tree authoring path requires selected tracked/admitted path coverage with no unreported failures. A partial-path commit must be explicitly requested and use the pinned base for unspecified paths. Stronger point-in-time assertions require quiescence, controlled writers, or a suitable filesystem snapshot. CI always tests a materialization of the newly committed **immutable** State, not whichever mutable files happen to be in the editor's directory later.

Recording a revision is separate from selecting it for a workspace or target. Prepare new blobs/tree nodes, hash/verify the closure, then perform a short resource transaction which checks the workspace's selected-revision generation and records the new immutable Revision plus its operation outcome. A concurrent selection change must not cause an old filesystem observation to be silently rebased onto the newly selected revision.

### Sparse and ephemeral authorship

A sparse client may not possess all source files, yet it can make an exact conditional edit to a complete StateId:

1. **Service construction:** send base StateId, changed object bytes, expected path/absence conditions and a scoped request ID. The service verifies the necessary namespace nodes and constructs the complete resulting root.
2. **Local authenticated construction:** a client holding suitable Merkle path and absence witnesses can recompute the root without downloading unrelated subtrees. The witnesses and canonical tree algorithm must be verified; this is a later optimization, not necessary for the first release.

A sparse client with *neither* the required proof nodes nor a reachable service can durably retain a **pending edit bundle** against its base, but must not claim it has produced a complete, fully verifiable new StateId. A full local repository can author and inspect states offline without promises.

If the target advanced after a worker started, that does not invalidate the worker's authored source. It creates a separate merge/integration question. A stale conditional edit against its *own stated base*, however, must fail rather than overwrite unknown content.

## 4. Recovery, authored history and accepted history have different clocks

Do not force one immortal revision DAG to store every editor keystroke, auto-capture or validation attempt. Define **three separately acknowledged resources**:

### A. Bounded private recovery

    Capture = {
      workspace_id,
      based_on: StateId,                 // pinned while required
      admitted_observations: Path -> CapturedEntry | Tombstone,
      coverage: { scanned_paths, skipped_paths, failed_paths },
      consistency: ObservedPerFile | QuiescedSnapshot,
      captured_at, retention_class
    }

The capture records selected **exact bytes** (including deletions and symlink targets) against a pinned base, and is reconstructible as a recovery view. It need not be a full semantic authored Revision or add an immortal history parent edge. Overlay/delta chains must be bounded; retained captures pin all dependent base data. A failed/unfinished capture is not acknowledged and must not become the new latest durable capture head.

**Trigger contract:** with no mandatory daemon, capture is attempted only by an explicit capture request or a **command/tool boundary explicitly configured to capture**. Read-only commands such as status remain read-only by default. Optional watcher/editor integration or agent tool-boundary hooks may offer more frequent captures but are independently observable and can fail. File-system watcher overflow, ignored/new files and incomplete scans must appear as coverage gaps and trigger reconciliation. A user cannot infer protection of unsaved editor buffers from AVC's history.

**Consistency contract:** ObservedPerFile means each included file's captured bytes are retained but the collection may never have coexisted at one instant. QuiescedSnapshot is allowed only if a suitable controlled writer boundary or filesystem snapshot establishes a coordinated view. Wall-clock time, double-stat and an advisory lock alone do not establish that property. A capture can be valid for recovering its selected bytes while not qualifying as a point-in-time workspace image.

**Durability contract:** acknowledging a CaptureId means its admitted content objects, base closure and capture root/pin are durably recoverable for the stated retention horizon **under the declared local or remote failure model**. It does *not* mean uploaded to a server, backed up off-device, a complete source tree, or retained forever. Status exposes last durable capture, age, selected coverage, skipped/failed paths, source-consistency class and remote-backup status separately.

**Retention:** expiring captures cannot be kept alive indefinitely through an append-only operation log. Operations may retain summaries but the content pins expire under explicit policy unless promoted into a named checkpoint, authored Revision or submitted draft. Reclamation and backup removal need their own tested semantics.

### B. Immutable authored revisions

A user/agent selects a complete source StateId and creates a Revision. It is independently inspectable, mergeable, and retainable, unlike a short-lived recovery overlay. It identifies **precise stored source**, not an unverifiable claim of what was simultaneously present in an uncontrolled live directory. Revision/source closure must be durable before acknowledgement; rewriting history creates new immutable objects and explicit evolution records.

### C. Proposal and shared acceptance

An integration candidate pins the exact source result, ancestry and merge recipe; validation evidence is about that candidate. Sharing restricted drafts does not automatically trigger public review, a CI run or target movement. Accepted targets advance by policy- and generation-checked transactions. A fast accepted-but-not-yet-verified target is a different explicit safety policy from required pre-acceptance validation.

## 5. Merge, typed conflict semantics and history evolution

A merge consumes exact immutable source States and ancestry, not an event log of keystrokes. It returns a **new State** and a separate *merge provenance* record describing the algorithm, source revisions, base calculation and resolution choices. It does not assume that changes in disjoint paths are programmatically independent.

Conservative tree merge:

    identical sides  -> common source entry
    left == base     -> right
    right == base    -> left
    both directories AND base directory/absent -> recursively merge children
    both directories BUT base another kind -> typed conflict if different
    otherwise        -> supported, pinned-version driver
                        or a typed unresolved source expression

Source conflicts can occur at a file, path entry, or entire directory subtree (delete/edit, type change, conflicting move, symlink-vs-directory, binary replacement). For multiple merge bases, compute an explicit conflict-preserving virtual-base or n-way procedure; **picking one arbitrary ancestor or concatenating nested conflict markers is not correct**. Each operation must be deterministic for fixed State inputs, versioned algorithm and resolution choices. No merging algorithm establishes that the composed program works.

### Separate what is unresolved from who caused it

A source **ConflictValue** contains the unresolved *source expression*: semantic base and alternative entry/subtree values, including absence. Canonical normalization collapses truly equivalent alternatives and orders/encodes alternatives in a documented deterministic way. It must preserve distinguishable alternatives from previous conflicted states rather than silently flattening them. The normalization algebra and hard multiway counterexamples block format stability.

A separate **ConflictOrigin** records the revision/path and origin side mapping, driver/recipe version and other provenance. These records are pinned by their *own referencing revision/candidate*. Conflict author labels are therefore not part of StateId. Two merge histories can produce one equal conflict-valued State but retain different origin records.

For a conflicted path, the source API returns a typed unresolved result: ordinary read/range operations must **not** return marker bytes as if they were the canonical file. Optional text marker generation is an explicit, lossy *workspace presentation*. Such files require a **conflict presentation manifest** binding the generated marker bytes and encoding to ConflictValueId/path; an unchanged marker file round-trips as the original typed conflict, not as newly authored literal code. Resolving it requires explicit confirmation or a validated marker parser that can distinguish arbitrary source text. Binary/namespace conflicts may have no sensible marker format. Never silently interpret editing or reformatting a marker file as resolution. Resolution creates new source entries, preserving the prior value/origin records for inspection; automatically replay a prior resolution only when its exact input conditions are verified.

### History editing: specify transformations rather than names

For an ordinary single-parent authored revision C with parent P, rebase onto new parent Q constructs:

    new_state = merge(base=State(P), left=State(Q), right=State(C))
    C' = Revision(state=new_state, parents=[Q], change=C.change, ...)

This is a *conservative three-way transformation*, which may produce a conflict-valued source. It is not a universal context-independent patch application or a promise of semantic correctness. Record Evolution(C -> C', Rebase). The old C remains immutable and recoverable under policy.

Rebasing a selected stack walks its chosen dependency/ancestry subset in topological order, using each already rebased parent as the next new base; ambiguous merges and descendant choices require explicit policy. **Do not silently rebase every descendant or rewrite a published target**. Rebasing multi-parent revisions requires separately specified parent mapping/merge handling; reject unsupported cases instead of treating them like linear commits.

**Amend:** construct a new immutable source state/revision with the same logical ChangeId and a recorded predecessor; retain divergent amendments. **Undo:** restore prior private resource root(s) under generation preconditions, with filesystem materialization handled separately. **Revert:** author a new inverse *content* change against current source using original parent/result, potentially creating conflicts. It is not guaranteed to causally remove just one origin of independently identical edits. **Reapply:** deliberately apply an inverse or previously reverted change under explicit current-context checks, even if the logical ChangeId appeared earlier. No global ever-seen ChangeId filter.

A RevisionId currently hashes immutable author/description metadata; consequently editing a description may rewrite descendant revision IDs. This is a legitimate cost rather than a hidden requirement. Before format freeze, evaluate separate versioned annotations/attestations *only if* empirical metadata-edit churn and offline annotation divergence make it worthwhile; source-state hashes must remain unaffected either way.

## 6. Workspace state: selected source, observations and actual files

**Critical invariant:** the database can select a Revision and pin captures, but cannot declare an ordinary directory perpetually clean. Other processes can write to it without passing through AVC. The filesystem is an *external mutable input*, not an independently maintained authoritative VCS root.

Maintain three independent durable facts per workspace:

1. **Selection:** currently selected RevisionId and generation. An immutable source intent, not an observation of disk.
2. **Capture head:** latest acknowledged CaptureId, its own generation, retention and coverage; a watcher capture does not advance or conflict with selection.
3. **Materialization record:** where AVC last wrote source, what source it intended, whether construction completed, and a directory ownership/generation token where supported.

A status call may produce a fourth **transient observation**:

    Observation = {
      workspace_id, selection_generation, observed_at,
      compared_paths, dirty_paths, new_paths, missing_paths,
      scan_failures, possible_watcher_gap,
      matched_selected_state_at_observation: bool | Unknown
    }

Do not persist an unqualified Clean flag as a promise about future bytes. "Clean at the last verified observation" and "capture protected" are distinct claims; a workspace can be dirty and fully captured, clean but never captured, or have a pending materialization with a valid selected revision.

### Operations and their effect on each authority

| Operation | Selection | Capture | Mutable files |
| --- | --- | --- | --- |
| status | Read | Read | Scan/compare; no implicit mutation |
| capture | Unchanged | Durable new pin if successful | Observe; never overwrite |
| finish authoring | CAS selection after pinning complete State | Remains independently recoverable | Does not overwrite current edits |
| switch to revision | Select separate workspace; original retained | Original captures remain | Materialize into a **fresh** directory by safe default |
| undo logical operation | CAS relevant selected resource(s) | Retain original captures | No implicit destructive checkout |
| restore capture | Unchanged unless explicitly selected | Read pinned capture | Recover into a fresh directory |
| accept shared target | Unchanged | Unchanged | No local materialization required |

During a capture the selected revision might change. The capture remains a real, separately pinned recovery record against its original base; it must not be silently retargeted. A new current capture head requires its own generation/selection preconditions. A pending capture cannot cause an unrelated workspace selection to fail merely by incrementing a global operation head. Every **acknowledged** capture gets its own retention pin even if its attempt to become the workspace's current latest-capture pointer loses a race. A concurrent selected-revision change may make that capture *historical*, never erase its acknowledged bytes.

### Safe materialization contract

A materialization has durable intent, destination identity, desired StateId, progress and outcome. Stages are **prepared → writing → verified/ready** or failed/pending-recovery. Readiness means the chosen destination's written bytes and entries were verified against source under documented filesystem/durability assumptions. On failure an incomplete destination remains marked incomplete; the original directory, its files and its captures are preserved.

**Fresh-directory restore/switch** is the portable safe default for uncontrolled external writers. Use an exclusively allocated destination and path-safe file creation; do not reuse arbitrary existing directories or follow symlinks outside the root. This can be accelerated by CoW/reflinks/overlay or VM snapshots but cannot require those technologies.

In-place updates may be offered only with **explicitly weaker** semantics or with a genuine writer-exclusion boundary. An atomic rename of a prepared path does not revoke a writer's old open file descriptor, and a check immediately before overwrite leaves a race. Preserving old directory generations and before-images is valuable for recovery but not proof of no lost **unobserved** edits. Do not claim otherwise on APFS, Linux, NTFS or network filesystems.

**Every independent writer gets its own writable directory** even when immutable content and approved caches are shared. Actor identity, process/session identity, selected revision and workspace ID remain distinct. A workspace is not a process, network or credential sandbox. A VM can be destroyed only after its source handoff is durably acknowledged if that work must survive.

Full repositories work offline. Sparse/lazy clients carry explicit promised-object state. Missing promises are errors or hydration events, never equivalent to path absence or empty files.

## 7. Derived indexes and tree data structures

Canonical persistent Merkle namespace objects are **source truth**. Other structures are derived and rebuildable:

- Revision-graph generations, skip links and selected ancestry/reachability indexes.
- Changed-path summaries, postings, negative Bloom filters, path/copy history.
- Blame and line-origin tables keyed by source and algorithm version.
- Optional symbol/AST indexes, search and change observation metadata.
- Physical fetch/object-set and cross-revision transfer indexes.

The initial reference tree may use sorted flat directory maps for correctness and realistic small-directory performance. **Before freezing the format**, compare deterministic bounded sharded/radix directory pages and ordered POS/prolly pages on huge/wide directories and rename/copy/diff workloads. Root hashing is defined by the chosen canonical format; caches may build independent query representations. A very wide unpaged directory could rewrite an entire directory blob per tiny edit, so do not mistake a toy flat directory tree for the final scalability solution.

Tree equality (matching canonical StateId) is fast. Single path read visits directory nodes and blob. Diff prunes identical subtrees. Copy/move of an unchanged subtree relinks immutable references. Text-level merge, detecting renames under ambiguous changes and validating combined program behavior still cost real work.

## 8. Storage authority and fault-tolerant transactions

The logical model **does not select a physical object store or a distributed consensus protocol**.

First implementation: one local durable SQLite transaction authority for canonical small objects, workspace roots, operation log, staging/pins and retry outcomes. It is a correctness baseline, not a permanent mandate. Store ordinary blobs directly initially; evaluate immutable segment files for large content or throughput when verified stage/flush-before-root and GC protocols exist. Set explicit WAL durability settings, handle full disk and long readers, and test crash stages.

Other candidates: append-only object+root journal or immutable compacted object segments with separately transactional root authority. Each must uphold the same guarantees: acknowledged roots have complete durable canonical closure; no in-flight object is collected; readers/backup see a coherent point; orphaned staged objects can be reclaimed safely. Domain-separated cryptographic content IDs validate bytes, not author or permission.

Physical placement, compression, delta coding, local numeric indexes and packed bundles **must not alter canonical source/revision identity**. Enroute's commit segments and dense IDs demonstrate one Git-specific physical optimization, not an AVC requirement. A locally assigned physical index number may be useful, but a globally coordinated sequential number cannot be necessary to create a source object offline.

Retention classes include expiring captures, named pins, independent draft roots, active readers/transactions, accepted history, and backups. GC preserves their reachable canonical objects. Deleted cached indexes must be reconstructible; deleted authoritative objects cannot.

## 9. Native source API, ephemeral submission, and publication

Define source operations over exact immutable IDs, usable by local library, CLI, full offline repositories and optional hosted service:

    read_entry(StateId, path) -> ResolvedEntry | TypedConflict
    read_file(StateId, path, range) -> Bytes | TypedConflict | MissingObject
    list_tree(StateId, path) -> BoundedEntries | TypedConflict
    diff(StateId, StateId)
    apply_edits(StateId, preconditioned_ops) -> StateId | SourceConflict
    record_revision(StateId, parent_ids, ChangeId, metadata) -> RevisionId
    rebase_single(RevisionId, new_parent, algorithm_id) -> RevisionId
    capture(workspace_id, coverage_policy) -> CaptureReceipt
    capture_status(workspace_id) -> ProtectionStatus
    observe_workspace(workspace_id) -> Observation
    advance_selection(workspace_id, expected_generation, revision_id)
    materialize(StateId, new_destination) -> MaterializationReceipt
    submit_draft(RevisionId, retention_policy, request_id) -> DraftReceipt
    propose(target, candidate) -> ProposalId
    land(target, expected_generation, candidate_id, evidence) -> Receipt

All requests must have explicit limits, typed incomplete/stale/conflict outcomes and stable retry keys scoped to authenticated epochs. A conflict read never silently emits marker text. A partial client knows which objects are promised and cannot represent missing data as an absent path.

### Ephemeral agent durability contract

An agent can derive source from a service-held base, run tools in a private container/microVM, and submit changed content without uploading all unchanged files. For a sparse agent that lacks sufficient Merkle path witnesses, the service checks base StateId and affected source preconditions, constructs and hashes the full result, and retains it.

**Uploading blobs is not a completed submission.** A successful DraftReceipt acknowledges **both** complete canonical content closure and a durable, owner-scoped **draft root/pin** with exact retention/expiry, within the service's stated failure model. The receipt includes resulting StateId/RevisionId, root ID, retention and retry identity. Only then may an ephemeral runtime be destroyed while promising that the work can be reconstructed. If upload/acknowledgement fails, the agent must retain local durable custody or export the unsubmitted bytes; do not claim protected handoff.

An offline sparse client without sufficient proof data may record a pinned **pending edit bundle**, not a fully authenticated global StateId. A full local repository can construct complete source states offline. Clients that have authenticated sibling/path witnesses can construct new roots locally when the tree format supports it.

### Transport and accepted integration

Support both batched/resumable bulk transfer of selected immutable source/history closures and sparse source/object queries with bounded cost. Physical bundles, layers, cache IDs and compression are replaceable transport/storage strategies, not canonical history. Shared caches must enforce authorization and limit cascaded cache-miss storms.

A candidate contains exact base and result StateIds, input revisions, explicit merge/driver recipe, target generation and policy version. Validation evidence refers to that candidate *and* runner/toolchain/test/external-input identities; candidate identity cannot contain its future evidence. Build tests run on an immutable materialization, not a mutable workspace still receiving editor writes.

Landing validates current target generation, policy, authorization and qualifying candidate evidence in **one authority transaction**, then records target change, durable receipt and event intent. Tests execute outside target locks. If target/policy moved, recomposition and revalidation may be required. Restricted draft sync, proposal, accepted and verified target movement have **separate** event and retention contracts. A host can intentionally allow accepted-before-verified staging, but cannot label it pre-acceptance verified.

For lost responses, an authenticated idempotency epoch and request digest select a previously committed outcome; delayed requests from expired epochs cannot be reinterpreted as new writes. External CI/deployment actions use reconciliation/outbox semantics, not an impossible promise that the VCS metadata transaction also atomically commits other services.

## 10. Decisions, unresolved algorithms, and proof obligations

**Firm product requirements:** a genuinely native non-Git VCS; full offline source; optional sparse/hosted execution; exact immutable source state; separate capture, authored revision, workspace selection and publication lifetimes; independent worker roots; explicit conflict values; safe recovery; source query/sync and backup APIs; no mandatory Git compatibility.

**Selected reference:** persistent source-root snapshots in an immutable Revision DAG; stable logical ChangeIds; separately retained conflict provenance; exact conditional source edits and resource-scoped transactions; best-effort/controlled capture fidelity labels; per-workspace selection and capture generations; fresh-directory safe materialization; durable private draft roots for ephemeral agents.

**Defined policy, implementation still required:** single-parent rebase as three-way state transformation; separate undo/revert/reapply; scoped idempotency and conditional landing. These rules are specification targets, not a claim the old Rust code implements them.

**Still format-blocking:**
1. Canonical Merkle directory paging and version/upgrade plan.
2. Canonical normalization and propagation of **multiway structural conflict expressions** without provenance contaminating source equality.
3. Portable file-mode/name/metadata scope and exact tracked/admitted path policy.
4. Whether immutable revision description/authorship belongs in revision identity or a separately versioned annotation/attestation; current reference includes it and accepts descendant-rewrite cost.
5. Snapshot-first versus authoritative changesets under the same complete workloads. Choose changed-file authority only if it *beats* this reference end to end, not because an incumbent's servers use it.

**Physical/scale optimizations, not logical blockers:** object segments/chunking, local page caches, tree-derived fetch indexes, remote service topology, optional in-place controlled workspace update. A causal patch algebra is a separate competitor only where it demonstrates substantial user value from causal selective undo/commutation.

**Never assert:** a filesystem capture is simultaneous without actual writer control; all dirty files are protected without an acknowledged capture; any content hash authenticates the actor; disjoint file changes ensure semantic compatibility; fast object upload is a durable draft without its root pin; a database CAS prevents external editor writes; or Enroute/Scion fleet metrics prove AVC performance.

No native engine or performance comparison has been implemented yet. Existing Rust source still uses a Git-command wrapper with a separate JSON log. Preserve it as a legacy prototype without attaching new native safety claims.

