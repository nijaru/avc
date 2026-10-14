# AVC: native VCS architecture

**Decision:** 2026-10-13 — native core; Git interoperability at the boundary.
**Status:** Proposed architecture, not implemented.

This supersedes the August native-storage blueprint and the intervening Git/Jujutsu-engine proposal. [Research](research.md) owns comparative evidence; [roadmap](roadmap.md) owns delivery gates. Previous versions remain in repository history.

## 1. Product and selection principle

Build a **general-purpose, local-first VCS with an independently designed source model, object format, transaction authority, workspaces, sync protocol and integration semantics**. Optimize for one developer and for many concurrent people/agents without requiring the latter to complicate ordinary work.

Most users already have Git; Jujutsu addresses much of Git's history ergonomics. A new VCS therefore needs **substantial improvements to complete workflows**, not merely faster commits or different command syntax.

Reuse proven components and ideas, not authoritative Git or Jujutsu state. Reusable hashing, compression, persistent-map, SQL, parsing and merge libraries remain appropriate. Own the contracts that determine what source state exists and how it evolves.

Core UX concepts: workspace, logical change, and shared target. Revisions, operation views, savepoints, proposals and receipts are inspectable underlying machinery. Editing and undoing must work offline on ordinary files. A server, agent, special filesystem, or complicated task object must never be prerequisite to standard source editing.

Not core responsibilities: agent scheduling, sandbox execution, CI, issue tracking, a full forge, editor collaboration CRDTs, or automatic LLM conflict resolution.

## 2. Three kinds of history

**Recovery history** captures admitted file states privately and frequently. A savepoint contains an immutable captured tree, workspace/base identities, generation, time, and capture completeness. It is retention-bounded and does not become another semantic ancestor or public event. Named checkpoints pin deliberately.

**Development history** contains logical Changes with stable random ChangeIds, plus immutable Revisions with content-derived RevisionIds, snapshot roots, semantic parents, descriptions and authorship. An amendment/rebase may create another revision of the same change; divergent revisions can coexist. Split/fold records explicit lineage without pretending one ID means several independent changes. An Operation graph records changes to repository views and supports undo/restore; undo appends an operation instead of mutating old source.

**Shared history** contains named targets, exact proposals/candidates and conditional publications. Review/validation evidence references the immutable candidate; it is not hashed into its subject. A publication records the selected target transition, applicable policy, authority, receipt, and at-least-once outbox events.

These histories have distinct durability and retention rules. Reverting a published revision is not equivalent to replaying private undo, canceling CI or revoking a deployment.

## 3. Snapshot truth with explicit change intent

**Canonical source truth is a persistent Merkle snapshot.** A revision points to an immutable tree; unchanged objects are shared, so this is not a physical copy of every file. A compact changed-path manifest, exact before/after blob identities and explicit change dependencies accelerate comparison and independent composition. The snapshot, not replay of an unbounded edit log, reconstructs any revision.

A directory graph preserves subtree identity across parent moves. Ordinary small directories use sorted flat entries. Very wide directories use **deterministic paged Merkle maps** selected by measured performance and history-independence tests. Benchmark canonical content-defined B-tree/prolly approaches, immutable search trees, and flat Git-style baselines. Insertion order must not change the logical result or cause pathological unbounded depth.

File manifests identify regular files, executable bit, symbolic links and content. Use whole blobs for source files. Support chunked large-content manifests only where realistic binary-edit workloads justify them; physical delta compression must not change canonical content identity. Preserve exact source bytes and fail explicitly where platform file names or metadata cannot be represented.

Conflict is a typed immutable object containing base, alternatives and provenance, not only textual markers. Revisions may retain unresolved conflicts; resolved Git export and publication cannot silently export them as ordinary files.

Pijul-like patch commutation is valuable to study, but not selected as universal source truth: a patch-only system makes arbitrary historical reconstruction and binary/rename semantics harder to verify. Independent file edits may compose cheaply under exact preconditions; that is not proof they are semantically independent. More advanced change algebra remains possible without making patch replay necessary for correctness.

Identity-bearing objects use versioned, domain-separated deterministic encoding and full hash IDs. Paths and map entries have canonical order; physical segment offsets, cache state, insertion order, and irrelevant timestamps do not influence IDs. Pin an algorithm and publish byte-level test vectors before format stabilization.

## 4. Native objects and transaction authority

Use an **immutable object store with append-only segments**, sealed immutable indexes and compaction, plus one **authoritative local metadata transaction store**. The metadata authority owns current repository/view roots, workspace generations, retention pins, idempotency results and event intents. Derived object-location, graph and path indexes can be dropped and rebuilt. The metadata authority itself is not a disposable cache.

Use SQLite initially for short metadata root transactions rather than inventing a WAL. Source payloads do not need to be written again inside the database. This is selective reuse of a proven transaction component, not adopting another VCS's source model. Respect SQLite's actual filesystem/durability assumptions; network filesystem support is not implicit.

Native source operation protocol:

1. Read immutable expected view and workspace generation.
2. Construct new immutable source objects and operation/view, allowing independent workers to prepare concurrently.
3. Make the complete referenced closure durable and GC-protected.
4. In **one short authoritative metadata transaction**, verify expected state and idempotency identity, select the new root, update relevant pins/outcomes and record event intents.
5. Commit durably and acknowledge only after that commit.

Before step 4 commits, prepared objects may be orphaned and reclaimed later. After it, the committed root must never reference missing objects. There are no independently authoritative Git refs or Jujutsu operation heads in native mode. Compaction swaps durable manifests atomically; live readers and in-flight operations protect referenced objects. All state transitions and crash points require model and fault-injection tests.

Do not hold a global transaction lock while agents run tests, editor processes edit, merges compute, or files are hashed. Initially serialize the short current-root CAS; scale with real contention measurements.

**Filesystem checkout is not part of a metadata transaction.** A materialization intent records old/desired tree, protected original bytes and progress. On crash, recover or resume safely; compare actual workspace contents before overwriting any file. Report committed source state and pending checkout separately rather than claiming both completed.

## 5. Recovery, local safety and privacy

Watchers, mtimes and file-size hints are optimizations, never complete truth. Overflow or uncertainty triggers reconciliation. Files changing during a scan may prevent a consistent capture; expose this honestly. Unsaved editor buffers are not protected by filesystem capture.

Auto-capture tracked/admitted source files, not every ignored or untracked file. Show new files as candidates for admission; never silently retain secrets, logs, caches or output artifacts. Ignoring a secret after capture does not erase prior retained copies.

Bound savepoints by age/count/bytes and transparently report last durable capture, coverage and policy. An immortal operation chain must not retain every automatic save. Retention roots include current views, published revisions, explicit pins, active readers, proposals, validation candidates, transfers, and in-flight imports. GC respects reader leases; interrupted collection cannot delete a committed closure.

Object integrity does not mean object confidentiality. Native remotes must authorize the complete reachable closure. Private recovery and draft data must not appear in default public export or upload operations.

## 6. Concurrent workers and machine interfaces

Independent workers receive **independent writable directories/workspaces sharing immutable objects**. This is source-file isolation, not a sandbox for processes, credentials, network access or external services. A stale process can keep writing a directory even if its API lease expires; never reuse that directory without separating or revoking physical access.

Workspace generations and owner/fencing tokens protect managed writes. Every mutation specifies expected revision/view ID and workspace generation when relevant. Concurrent modifications explicitly diverge or return a stale-precondition error; no last-timestamp-wins rule.

The canonical library API works **without a checkout**: read tree/file/range, diff and history; create/edit revisions with exact expected blob IDs; inspect/resolve conflicts; compose changes; create candidate/proposal; inspect validation; request integration. CLI, local structured requests and optional remote RPC use one semantic implementation. Queries are bounded and paginate over an immutable specified view.

Machine requests include repository, schema, operation, exact inputs, preconditions, limits and idempotency identity. Requests apply all source edits or none. Duplicate accepted requests return their recorded results. Scope IDs by authenticated actor and authority-issued epoch; reject retired epochs rather than replay an expired unknown write. Cancellation after commit cannot undo the recorded result.

Optional input observations (read blob hashes, directory listings and negative lookups) help diagnose stale agent reasoning. Incomplete observations are advisory. Even complete read sets cannot establish semantic program compatibility. Explicit dependencies differ from observed inputs.

## 7. Merge and integration

Start with deterministic tree/three-way textual merge and typed conflicts for deletions, type changes, binaries and nested conflicts. Preserve user bytes and offer conservative fallbacks. Optional versioned syntax-aware merge drivers can improve results but are not an LLM authority or a correctness proof.

Separate four operations:
- **Recover:** local private durability; no network or CI.
- **Sync draft:** opt-in restricted backup; no public project events by default.
- **Propose:** expose selected exact revisions for review/checks.
- **Land:** conditionally advance a shared target using exact validation/policy.

An immutable candidate records target, expected base/generation, selected revisions/dependencies, resulting source root, merge/driver identity, policy and validation recipe. Evidence and approvals refer to this candidate; acceptance refers to chosen evidence; publication receipt refers to acceptance. Never construct circular self-hashing manifests.

Run checks against that *exact* materialized candidate. A changed base or policy requires appropriate recomposition and revalidation; disjoint modified files do not prove combined correctness. A linear target imposes some integration order. Batched landing is permitted if the final batch candidate is validated; that does not certify every intermediate revision.

Native sync transfers requested immutable objects via resumable batches/ranges, validates hashes/closure/quotas in quarantine and supports bounded caches. Remote publication performs target-specific CAS plus receipt/event outbox under authoritative policy and revocation checks. Delivery may repeat; stable event IDs enable deduplication. Keep ordinary development fully offline-capable.

When integrating with an existing Git host, **that host** controls branches, reviews, policies and CI. Reuse its merge queues and protections. An AVC-local record cannot atomically commit third-party host state; adapters must reconcile timeouts and label weaker guarantees instead of pretending the native contract applies.

## 8. Git import/export, not Git as the store

Import existing Git commits and trees, tracking exact original object IDs and author/parent metadata. Export supported **resolved** native histories into actual Git objects for existing remotes. Unchanged imported objects may preserve their original bytes/identity; rewrites and native-only metadata cannot always round-trip. Explicit mappings make changes visible, not secretly lossy.

Support common Git workflows early, but test a real feature matrix: ignored files, .gitattributes and EOL/filters, executable/symlink semantics, path encodings, signatures/tags, LFS, submodules, shallow/partial clones and external refs. Refuse unsupported operations before source damage. No silent sharing of the Git index, no automatic pushes of private saves, no multiwriter colocated mutation in the initial bridge.

The bridge has real cost and incomplete semantics. Existing-host adoption is a product requirement; reproducing all of Git is not an excuse to compromise the native core.

## 9. Data trust and correctness obligations

Untrusted objects, names, paths, source configuration and transport responses must be validated without executing repository-provided code. Limit decompression and allocations; protect against traversal, symlink escapes, case collisions and malformed graph references. Remote credentials and publication authority are distinct from a worker's chosen name.

Release gates:
1. Acknowledged operations have durable, complete reachable objects and stable retry outcomes.
2. Incomplete source/metadata transactions remain invisible, including after every tested crash point.
3. Stale workers cannot overwrite current managed state without explicit reconciliation.
4. Checkout/restore/undo never silently destroy later user edits.
5. Retention and GC cannot delete live or reader-held roots; expired savepoints are reclaimable.
6. Auto-capture does not leak private data into public history.
7. Candidate validation and publication authority bind to exact source/policy identities.
8. Git exchange preserves its declared supported subset; unsupported features fail clearly.
9. Derived indexes never become a second authority.
10. Readers and writers stay correct under concurrency and untrusted input.

A clean unit-test run is not production confidence. Use property/state-model tests, differential fixtures, fuzzing, process-kill fault injection, realistic power-loss/FS qualification and independent backups. Experimental status remains explicit until those are established.

## 10. Performance choices that must be earned

The native **logical** model is a decision. Specific large-directory paging, chunk thresholds, segment policy, indexing, lazy working-copy backend and remote cache topology are not yet empirical facts.

Compare tuned Git and current Jujutsu against the native system on ordinary software projects, giant directories, binary churn, long histories, 1/8/32 concurrent workers, repeated automatic captures and end-to-end validated integration. Measure not just revision latency but CPU, RAM, bytes read/written, retained storage, recovery time, network traffic, user steps, and review/CI events. Retain exact benchmark hardware and seeds.

The aim is a general-purpose improvement; don't sacrifice ordinary small-repository behavior to win an exotic fleet workload. Design freedom is an opportunity, **not evidence of an achieved speedup**.

## 11. Effect on previous design

**Retain:** local-first, immutable snapshots, stable changes, operation-history undo, structured conflicts, bounded private recovery, isolated workspaces, quiet draft synchronization, deliberate validation and publication.

**Replace:** Git/Jujutsu as canonical runtime; two authoritative heads bridged by adapters; Git-host-first semantics driving the local model; a mandatory complex server.

**Re-evaluate by experiment:** flat versus paged directory maps, patch commutation, selective chunking, virtual materialization, distributed caches and new physical compression.

Start implementation with the native transactional snapshot core and rigorous integrity tests, not additional wrappers around the old Git auto-commit implementation.
