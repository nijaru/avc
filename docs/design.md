# AVC architecture: independent work, composable source states

**Design iteration:** 2026-10-14  
**Status:** Proposed native architecture; not an implemented or benchmarked system  
**Scope:** A general-purpose, local-first VCS suitable for solo development, collaborative teams, and concurrent agents

This is a clean-sheet model, not an attempt to combine Git, Jujutsu, Scion, Pijul, and others feature-by-feature. [Research](research.md) evaluates alternatives and primary sources; [roadmap](roadmap.md) defines experiments and release gates. The old Git-backed Rust prototype has none of the safety properties claimed here.

## 1. Design goal and irreducible requirements

Optimize **develop → preserve → compose → verify → share**, including abandoned and speculative attempts. A user should be able to work safely and offline in an ordinary directory. More workers should make independent progress without unnecessary mutual coordination or repeated full clones. A small project must not pay for an enterprise-scale service.

A useful VCS must answer distinct questions:

1. **What are the exact bytes?** Any named source state must reconstruct unambiguously, including opaque files and invalid or incomplete programs.
2. **What work was authored?** Authored intent and its evolving versions should retain identities independent of where they are eventually applied.
3. **What depends on what?** Causal dependencies, observed inputs, ancestry, and replacement history are different relations.
4. **What was combined?** A result must identify its inputs, transformations, conflict decisions, and exact resulting source state.
5. **What is accepted?** Shared targets have explicit conditional transitions and applicable validation/authority.
6. **What can be recovered?** Local intermediate work is protected without turning every editor save into a permanent semantic change or remote event.
7. **What is actually known?** A textual merge, successful file-level CAS, and a passing test attest to different properties. None establishes arbitrary program correctness.

Safety and comprehensibility are constraints, not negotiable performance dimensions. Optimize latency, storage, bandwidth, and coordination only within the correct semantics.

**Core decision:** build a native system with source-state identity, immutable authored work, context-specific applications, independent workspace roots, and accepted target transitions. Git is an interchange format, not the canonical store. Existing algorithms and libraries are eligible for reuse, not mandatory architectural ancestors.

## 2. The minimal conceptual model

The ordinary user works with **a workspace**, **a change**, and optionally **a target**. Internally, these map to six different concepts:

| Entity | Identity | Meaning |
| --- | --- | --- |
| **State** | Content-addressed StateId | Exact source tree, including typed unresolved conflicts where necessary; no history or author implied |
| **Work** | Random, stable WorkId | Logical unit of intent; a named task/change, not a branch or a filesystem checkout |
| **WorkVersion** | Immutable content-addressed VersionId | One authored version of a work unit, with authored base state, resulting state, predecessors and explicit dependencies |
| **Application** | Immutable content-addressed ApplicationId | Applying selected WorkVersions against a specified state, including transformation choices, conflicts, and resulting state |
| **Workspace** | Stable WorkspaceId plus generation | One mutable working environment, active work identity, protected recovery, and independent current source root |
| **Target** | Named target plus generation | One accepted shared state with publication history and policy; moving it is an explicit conditional transaction |

Other records: a **recovery capture** is a private, retention-bounded observation; an **operation** is a durable state transition used for undo and audit; a **proposal** asks to review an exact application; **validation evidence** and **acceptance** refer to that immutable candidate; a **publication receipt** records a completed target transition.

### 2.1 State is not a commit

State is a hierarchical map of canonical path components to source entries. Entries can denote regular file content plus executable bit, symlink target bytes, directories, or typed unresolved conflicts. The root is a content-addressed commitment to the exact logical tree. It contains **no author, timestamp, branch, ancestor, or agent transcript**. Equal source trees have equal StateIds regardless of the path taken to create them.

The State graph is a persistent Merkle directory map: immutable content and equal subtrees are shared. Do not confuse a snapshot with a physical copy of the entire repository. A State object may be stored in segments, as a database value, or retrieved remotely; its identity must not depend on physical placement.

### 2.2 Work is not its application

WorkVersion records an authored **base State**, **result State**, predecessor WorkVersions of the same WorkId, and explicit dependency WorkVersions. The transformation from base to result is derivable exactly from those states; optional changed-path manifests, explicit move/hunk intent and observed inputs accelerate safe application but cannot contradict the committed result.

There can be concurrent divergent WorkVersions under one WorkId. A newer timestamp doesn't choose the winner. Split/fold create explicit many-to-many lineage; they are not silently the same identity as their components. Revisions of authored work are immutable even after integration.

**Applying work to a different base does not rewrite the authored WorkVersion.** Instead an Application records the target base, selected exact WorkVersions, ordering/dependency choices, merge algorithm and version, any conflict resolution, and the resulting State. Different applications can reuse the same authored work against different contexts. An actual intentional edit to a work unit produces a new WorkVersion.

This distinction prevents an advancing shared target from forcing a cascade of rewritten authored versions merely to describe where work was integrated. It also gives review and provenance an immutable authored subject. It does *not* eliminate the cost of computing composition or validation on a changed base.

### 2.3 Example

~~~text
S0: shared source state

agent A authors work WA-v1: S0 -> SA
agent B authors work WB-v1: S0 -> SB

compose(base=S0, work=[WA-v1, WB-v1]) -> application C1, state SAB
publish(target=main, expected=S0, candidate=C1) -> main=SAB

later base S1:
compose(base=S1, work=[WB-v1]) -> application C2, state SB-on-S1

WB-v1 remains the same authored work in both C1 and C2.
C1 and C2 record different context-specific applications.
~~~

A user can still have a familiar linear display of accepted changes. Internally, accepted target ancestry and authored-work dependency are distinct relationships. A deterministic Git export may synthesize linear or merge commits representing a selected target history, but cannot always preserve native IDs or all provenance.

## 3. Composition and causality

Composition is the architectural center, not branch rebasing. Its inputs are immutable: an exact base State, exact WorkVersion IDs, an explicit dependency closure, the selected merge policy and any resolutions. Its output is one immutable Application and State (possibly conflict-bearing).

Algorithmic contract:

1. Verify each WorkVersion's stored base/result relationship and dependency closure; reject missing or inconsistent objects.
2. Check application provenance before altering source. The same exact WorkVersion must not be inadvertently applied twice to one candidate/target; content equality alone cannot prove prior application. For one WorkId, selecting incompatible divergent WorkVersions requires an explicit selection or a separately authored resolution, not timestamp precedence. Dependencies on exact predecessor versions must be satisfied or **explicitly substituted/reconciled**; a clean textual merge cannot silently satisfy a stale declared dependency.
3. Choose a deterministic dependency-respecting plan. If two changes are incomparable but order affects outcome, report that ambiguity or require an explicit plan; do not invent an author-intent ordering from timestamps.
4. Prefer exact object/path preconditions for cheap application. A disjoint, type-safe path update is mechanically composable, including deletion and modes under clearly specified rules.
5. For changes that overlap or whose context is stale, compute a deterministic three-way/multiway merge (base, authored result, current candidate); preserve all alternatives as typed conflicts if unresolved.
6. Produce the exact resulting State and an Application record. Any changed-path hint or cached resolution must be verified against the final State.
7. Independently validate the composed candidate for integration under the applicable policy. Clean textual composition is **not** semantic independence.

A completed Application must be reproducible from immutable inputs plus pinned deterministic algorithm/driver identities and resolution data. It records the exact applied-work provenance needed to avoid accidental duplicate integration, and a target's accepted lineage is queryable independently of coincidentally equal source bytes. External LLM calls or mutable services can *propose* a resolution but are never implicit inputs to canonical object decoding or replay.

### 3.1 What commutes?

For two effects A and B, true context-independent commutation means there are valid transformed forms A' and B' preserving work identity such that:

~~~text
apply(apply(S, A), B') = apply(apply(S, B), A')
~~~

Identical end bytes alone do not establish that the *intended* changes were preserved. This relation is not automatically satisfied by ordinary file diffs. A disjoint path CAS can commute mechanically if directory names, types and exact read/write preconditions do not interfere. Nearby text edits, renames, delete/recreate, copies, binary replacements, and structural edits can be ambiguous.

**Decision:** use a state-anchored transformation/composition model with explicit Work identity. Investigate a genuine patch algebra as a competing **canonical** source model during Phase 0, not merely as an optional speed trick. If patch algebra can preserve identity/conflicts, work on opaque data, and reconstruct arbitrary historical states better, revise the core before format stabilization. Do not claim Pijul's commutation guarantees while actually implementing Git-style three-way merges.

### 3.2 Conflict states and resolution reuse

A conflict object retains source base, alternative terms, file/path/type changes and provenance; it is not just marker text in a working directory. Conflicting State roots remain valid for private work, inspection and further composition; unresolved source cannot be silently declared buildable or exported as ordinary Git content.

A resolution is an authored transformation against an **exact conflict identity** and explicit surrounding context. Reuse it only when the new conflict matches the conditions it declares. A “resolved once” rule is not automatic when a base or input version changes. Preserve prior alternatives and make losses of intent visible.

### 3.3 Different relations must remain different

- **Source equality:** same StateId, regardless of how reached.
- **Authored evolution:** WorkVersion predecessor/successor/divergence.
- **Dependencies:** one WorkVersion declares prerequisite versions or constraints.
- **Observed inputs:** optional file/object/directory reads, including negative lookups; may be incomplete and cannot certify semantic independence.
- **Application provenance:** exact selected WorkVersions plus context-specific resolution/result.
- **Accepted history:** target state transitions, with receipts and policy.

A single parent-commit graph cannot faithfully replace all these relations. Do not create another globally mutable “latest revision” to hold them together.

## 4. Workspace and operation semantics

Every independent worker edits its **own mutable directory or virtual workspace**, sharing immutable content and caches, not dirty filesystem state. One authenticated actor may have many sessions and Workspaces. File ownership and session IDs are not the same as actor identity. Do not turn a VCS workspace into an assumed process, credential, service, or network sandbox.

Each WorkspaceRoot is versioned independently: workspace ID, generation, current State, active WorkId, latest accepted WorkVersion (if any), and protection/materialization state. A mutation checks that workspace's expected generation and exact source inputs. Unrelated workspaces can advance without logical CAS conflicts. Short physical commits may still be serialized within one local storage engine.

No **one global operation head** should be moved by each background save or independent worker. Instead, durable operation transitions identify the resources/generations they affect, link to earlier transitions on those resources for undo, and contain inverse or previous-root information. A multi-resource operation checks/updates all affected roots atomically. A repository-wide coherent view can be *queried* at a transaction sequence/generation; it need not be one mutable object rewritten on every edit.

Undo appends a new authorized transition restoring prior logical roots if current preconditions permit. Published targets require new publications/reverts, not private undo. Restoring logical source state is separate from materializing a filesystem and cannot reverse external CI, messages, or deployments.

Named branches/bookmarks are optional labels on work or local roots; targets represent intentionally accepted shared states. Do not require a public branch per agent or exploratory attempt.

## 5. Recovery is separate from authored history

Normal editing must not create a permanent semantic WorkVersion on every file change. Automatic capture records private, bounded **recovery observations** (base State or prior capture plus changed content, per-path outcome, generation and capture status). A compact append of file-content references and changed paths can be cheaper than hashing a complete immutable tree for each keystroke; materialize a stable State when necessary for an authored version, named checkpoint, handoff or validation.

This is a recovery representation, not a second independent source authority: reconstruct a capture deterministically against its pinned base; check content identity and retained references. It may be pruned under published time/space policy. Named checkpoints pin selected recoverable states explicitly.

Filesystem notifications, mtimes and editors are hints, not truth. A scan can mix files from different instants when other writers continue editing. Mark it best-effort; never label it an atomic multi-file capture unless the writer quiesced or a suitable filesystem snapshot/controlled API established that property. Unsaved editor buffers are outside filesystem protection.

Capture tracked or explicitly admitted source files. Discover new files without silently storing ignored files, caches, or secrets. A previously captured secret remains in prior retained history until properly purged; ignoring it later is not erasure. Show last durable capture time, coverage, failures and retention horizon.

**Critical rule:** a durable repository transaction and a mutable checkout are separate things. A restore/switch stores its intended old/new state, protects original file bytes, compares *current* filesystem contents immediately before any overwrite, and reports incomplete/pending materialization on interruption. Never replay an old operation onto unobserved newer edits.

## 6. Storage engine: logical decision, physical competition

**Selected logical storage:** immutable typed content-addressed objects; deterministic canonical encodings with version and domain separation; one authoritative durability boundary for mutable resource roots; rebuildable derived indexes; retention and GC based on reachable pinned roots.

**Not yet selected:** the physical object store or local journal. Three credible designs must compete *under identical logical and durability contracts*:

| Candidate | Strength | Difficulties to prove |
| --- | --- | --- |
| One append-only object/transaction journal, derived indexes | Object data and root-commit record in one durable stream; strong sequential write properties | Recovery parsing, segment rollover, multiwriter ordering, compaction, reader epochs |
| Immutable object segments + authoritative SQLite root/metadata transaction | Mature short metadata CAS, source objects outside its WAL | Ordered object fsync before root CAS, staged-object GC races, cross-file backup/recovery |
| SQLite small objects + roots; external large blobs | Simple small-object atomicity and backup, fewer separate small-file write steps | WAL/checkpoint amplification, large-object ordering, contention, long-history size |

Choose by crash correctness, full workflow latency, write amplification, retention/GC, recovery and maintenance—not by whether the code has a familiar database name. Physical storage does not determine StateId, VersionId or ApplicationId.

GC must coordinate *in-flight prepared objects* and readers with compaction, not merely mark currently visible roots. Make complete object closure durable before an acknowledged root points to it. Segment retirement waits for reader/transaction epochs. Backups contain a **consistent authority and all referenced immutable objects**, not just a copy of the root DB or caches.

For hierarchical source trees, sorted flat small directories are a baseline; use canonical paged Merkle maps when wide-directory benchmarks establish a benefit. Test equal-content roots across insertion orders and adversarial path distributions, including shifted page boundaries. Whole blobs are the default; chunking and physical deltas are conditional on real large-content workloads. Hash choice (e.g. BLAKE3-256 versus SHA-256) and exact byte encoding require pinned vectors, migration rules and a security review.

## 7. Efficient queries and optional materialization

Every core operation must work against immutable State/Work/Application IDs without a checkout: read file/range/tree, compare states, inspect ancestry, list changed paths, determine dependencies, construct work, apply work, inspect conflicts and build an integration candidate.

Derived structures—path history, blame, ancestry generations/skip indexes, changed-path indexes, rename/copy candidates, perhaps syntax/symbol data—are keyed by immutable input roots and algorithm/configuration versions. Their loss affects latency only; authoritative source and operation answers remain reconstructible. Compute lazily or eagerly by measured demand, recursively and incrementally where possible.

Ordinary full local repositories are self-contained offline. Sparse/lazy clients may possess a **promised** incomplete object set, but missing promised content is never silently treated as deletion or empty input. On-demand queries, shared node cache and remote cache tiers can accelerate enormous hosted monorepos without changing logical semantics. Physical materialization can use ordinary directories, reflinks, sparse checkouts or optional virtual filesystems; no custom filesystem is required for safe basic operations.

## 8. Sync, review and publication

Separate four independent boundaries:

| Action | Meaning | Default side effect |
| --- | --- | --- |
| Capture | Recover local unfinished edits | No public event, no network |
| Draft sync | Opt-in protected transfer of selected private work | Restricted roots; no public review/CI |
| Propose | Present exact authored WorkVersions and a composed candidate | Review/validation as configured |
| Publish (land) | Advance one accepted shared target if exact conditions hold | One target transition and audit/outbox event |

Native sync exchanges immutable objects and declared roots with resumable batching, verification and authorization over the reachable object closure. Do not require a full clone to answer a query, and do not force a network request per tiny object. Local user operation remains possible without the remote service.

A Candidate references an exact base State and target generation, ordered/dependency-respecting selected WorkVersions, application result State, deterministic driver/recipe identifiers and policy epoch. Test results, reviews and attestations refer **to** that Candidate; they are not included in its own content hash (avoiding self-reference). An Acceptance references qualifying evidence and subject; a publication receipt references Acceptance and the committed target transition.

For a managed server, target CAS, current policy/authorization and receipt/outbox are one authority transaction. The target generation is updated **per target**, not under a global repository lock. Tests run outside locks against immutable candidates. If base, candidate, policy, or relevant required inputs change, recompose and revalidate as required. A merge queue may batch multiple work units and validate the combined State; that does not certify every possible intermediate ordering or semantic correctness.

There is no universal free solution to shared target ordering: a single linear target has an essential serialization point. Optimize *accepted validated changes per unit cost/time*, not merely raw change-creation rate. Some organizations may choose post-merge/pre-deploy verification; express the weaker guarantee explicitly instead of claiming it is equivalent to pre-publication validation.

External Git hosts own their own branch protection and review/CI policy. An AVC client cannot atomically enforce a third-party host's state in a local transaction. Export/import adapters must use host guarantees and reconcile unknown outcomes, never invent stronger ones.

## 9. Git interchange and broad project use

Git should be supported as an explicit translation boundary, **not** the canonical repository engine. Import Git source/history with mappings to original Git object IDs and metadata. Export selected resolved source states plus accepted lineage to valid Git history and existing review services. A synthetic Git commit/merge may be required to represent one native multi-work application; document losses of native WorkVersion/Application identity, alternative states and conflict metadata. Original immutable Git objects may be preserved where actually unchanged; never promise hash preservation after conversion/rewrite.

Build and test a compatibility matrix covering path bytes/Unicode/case collisions, ignore rules, executable bits, symlinks, attributes/EOL/filters, tags and signatures, LFS, submodules, large binaries, shallow/partial repositories and nested tooling. Refuse unsupported mutation/export **before** losing data. A native writer must not race an ordinary Git writer on the same mutable checkout/index.

Native source truth must not depend on a package manager, language parser, AST, model, agent logs, or build environment. Those are optional derived tools. Nonmergeable assets may use explicit, server-enforced publication leases; offline drafts remain possible but cannot claim online exclusivity.

## 10. Safety and correctness invariants

**Release-blocking invariants:**

1. Equal logical State content always yields equal StateId across physical stores and update orders.
2. Every WorkVersion's base/result/dependency closure is complete; optional effect metadata cannot contradict its result.
3. Application output reconstructs deterministically from exact recorded inputs, pinned algorithms and resolutions, with every unresolved alternative preserved.
4. Acknowledged root mutations have complete, durable, reachable objects and stable idempotent retry outcomes.
5. Concurrent independent workspaces can advance without falsely conflicting on one global logical head; overlapping resource updates cannot silently overwrite.
6. Recovery, undo and interrupted filesystem materialization never clobber newer unobserved files; status may explicitly remain pending.
7. Expired automatic captures become reclaimable, but GC/compaction never deletes live, pinned, reader-held, in-flight or published objects.
8. A private capture never implicitly triggers public notification, CI or upload.
9. Candidate evidence and policy bind to the exact State/Application and authority at publication time.
10. Derived indexes can be deleted and rebuilt without changing answer semantics.
11. Git exchange preserves its *declared supported subset*; unsupported semantics error rather than silently corrupting source.
12. Offline operation and remote divergence preserve both sides instead of using timestamps as conflict resolution.

Model and fault-test these properties, including malformed/untrusted objects, API request replay, crash at every durability edge, GC races, path/symlink attacks and cross-platform filesystem behavior. An integrity hash detects some corruption, but is not an independent repair mechanism. Independent backups and inspectable exports remain necessary during incubation.

## 11. Concrete scenarios to falsify the design

- **Disjoint files, broken API:** one agent changes a public API; another edits a separate caller based on the old API. Composition can be mechanically clean but candidate validation fails or stale-read observations warn.
- **Dependency evolves:** B depends on A-v1 while the target contains A-v2. B's original WorkVersion remains immutable; the Application must reject an unsatisfied exact dependency or explicitly record and validate an authorized substitution. It may produce a transformed application without pretending B's authored version changed.
- **Two variants of one work:** agents concurrently author A-v2 and A-v3. Both remain visible; composing both requires explicit divergence resolution, not last-writer-wins selection.
- **Duplicate integration:** the same WorkVersion is selected for an accepted target twice. Source equality or textual idempotence cannot hide duplicate application; the application planner consults recorded accepted-work provenance and rejects or explicitly treats a repeated version as already applied.
- **Rename versus edit:** an editor renames a file as another worker edits the old path. Optional explicit move provenance helps, but ambiguous filesystem inference must not silently drop content.
- **Crash between root commit and checkout:** the source transaction is durable, the materialization remains pending, and newer editor bytes are protected.
- **A million captures:** recovery retention remains bounded without leaking source objects through immortal operation views.
- **Offline target divergence:** both accepted remote/local target proposals are preserved until an explicit reconciliation or a proper target CAS chooses one.
- **High-rate agent integration:** batching reduces shared target transitions but does not substitute for candidate-based tests or reviewer policy.

A numerical performance claim only counts after a reproducible workload comparison against maintained Git, modern Jujutsu and relevant alternatives.

## 12. What is actually decided

**Selected:** native architecture; independent State/WorkVersion/Application identities; local-first source state; separate bounded recovery; per-workspace and per-target root authority; checkout-free API; typed conflicts; candidate-based publication; Git translation boundary.

**Strong working hypotheses:** snapshot-root source representation with verified change metadata; small flat directories with deterministic paged large maps; whole ordinary blobs; conservative three-way merge supplemented by exact-path composition.

**Open to replacement:** the canonical patch/state relationship if a real commuting-change model outperforms it; physical object/journal design; tree paging algorithm; hash and canonical codec; chunk/delta policy; recovery overlay encoding; lazy filesystem strategy; first-release Git interoperability subset.

This is a single product model designed from target workflows and correctness requirements, not a commitment to reproduce any particular existing VCS internals. The next iteration must use independent prototype evidence to resolve the open physical and patch-algebra questions.
