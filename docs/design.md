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

Separate **one authoritative logical transaction mechanism** from immutable source objects and disposable indexes. Current root selection, workspace generations, retention pins, accepted request outcomes and publication intents must have one durability/visibility authority. Object-location, graph and path indexes may be rebuilt; root/operation authority may not silently disappear with a cache.

**The previous append-only segments plus SQLite combination is a candidate, not a finalized physical design.** Compare three real implementations before freezing it:

1. A native append-only transaction journal with immutable object records and durable root-selection records; SQLite, if present, holds only rebuildable indexes. One durable append boundary, but greater responsibility for log recovery, compaction and concurrent transactions.
2. Immutable object segments plus an authoritative SQLite WAL database for root and coordination transactions. Supports expressive CAS/metadata queries and avoids copying every blob into the WAL, but requires correct object-sync-before-root-commit ordering and protects objects prepared during concurrent GC.
3. SQLite-backed small objects and root metadata committed together, with separately durable large-object storage. Strongest simple atomicity for common small updates, but evaluate WAL write amplification, large repositories and backup/checkpoint cost.

Prefer the simplest design that meets durability and full-workload performance gates, not the least number of named components. SQLite's FULL synchronous mode in WAL is materially different from NORMAL for power-loss durability; also respect filesystem and network-share constraints. The original native single-append-journal idea deserves direct evaluation rather than being discarded merely because SQLite exists.

**Avoid a single logically mutable repository-wide head for all workers.** The authoritative state contains independently versioned workspace roots and public targets. Workspace A advancing does not invalidate B's unrelated write. A shared change, named target, or multi-workspace transition still requires the appropriate exact preconditions and one atomic coordination transaction. One local database writer or append-log sequencer may serialize short physical commits; that is separate from *logical* conflicts between independent workers. A repository-wide view can be computed for queries or exported as a snapshot, not imposed as the one head each local edit must move.

Native source operation protocol:

1. Read the immutable input revision and the **affected** workspace/target generations.
2. Construct new immutable source objects and operation records concurrently with unrelated workers.
3. Protect prepared objects from GC (for example with registered staging segments/leases); durably write the referenced object closure and any required segment/manifests.
4. Commit one authoritative root transition: check only the affected expected generations, select new workspace/target roots, and record pin transitions, idempotency outcome, and required events.
5. Acknowledge only after that transition is durable. A lost reply must resolve to the original accepted outcome on retry.

Before step 4 commits, prepared objects may be orphaned and reclaimed later **only after in-flight transactions and references are reconciled**. After it, the committed root must never reference missing objects. A GC snapshot must not race a newly prepared and committed object into deletion. There are no independently authoritative Git refs or Jujutsu operation heads in native mode. Compaction publishes fully durable new segments and manifests before retiring old ones after reader/in-flight protection; it must preserve old or new valid state on interruption. The root authority and immutable closure must be included together in backup/restore; losing an authoritative database is not equivalent to dropping an index. Test every transition and failure point.

Do not hold a global transaction lock while agents run tests, editor processes edit, merges compute, or files are hashed. Initially serialize the short current-root CAS; scale with real contention measurements.

**Filesystem checkout is not part of a metadata transaction.** A materialization intent records old/desired tree, protected original bytes and progress. On crash, recover or resume safely; compare actual workspace contents before overwriting any file. Report committed source state and pending checkout separately rather than claiming both completed.

## 5. Recovery, local safety and privacy

Watchers, mtimes and file-size hints are optimizations, never complete truth. Overflow or uncertainty triggers reconciliation. Files changing during a scan may prevent a consistent capture; expose this honestly. Unsaved editor buffers are not protected by filesystem capture.

Auto-capture tracked/admitted source files, not every ignored or untracked file. Show new files as candidates for admission; never silently retain secrets, logs, caches or output artifacts. Ignoring a secret after capture does not erase prior retained copies.

Bound savepoints by age/count/bytes and transparently report last durable capture, coverage and policy. An immortal operation chain must not retain every automatic save. Retention roots include current views, published revisions, explicit pins, active readers, proposals, validation candidates, transfers, and in-flight imports. GC respects reader leases; interrupted collection cannot delete a committed closure.

Object integrity does not mean object confidentiality. Native remotes must authorize the complete reachable closure. Private recovery and draft data must not appear in default public export or upload operations.

## 6. Concurrent workers and machine interfaces

Independent workers receive **independent writable directories/workspaces sharing immutable objects**. This is source-file isolation, not a sandbox for processes, credentials, network access or external services. A stale process can keep writing a directory even if its API lease expires; never reuse that directory without separating or revoking physical access.

An authenticated person or agent can control multiple concurrent sessions/workspaces; **actor identity, session identity, workspace identity, and ownership are separate**. File ownership observations and provenance must not identify workers solely by the actor name, nor require scanning all other workers' operation logs on every write. An ownership hint does not enforce filesystem isolation; a guard is only strong when applied at the actual mutation boundary.

Workspace generations and owner/fencing tokens protect managed writes. Every mutation specifies an expected immutable revision plus the **specific resource** and expected generation. Different workspaces can advance independently; two updates to the same change/revision or target explicitly diverge or return a stale-precondition error. No last-timestamp-wins rule. A logically shared operation that intentionally updates multiple resources validates and commits them together. Metadata locks must not be held while running agents, merges, or CI.

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

## 11. Source identity, renames, and portability

Paths are logical names, not reliable permanent file identities. Stable file IDs could improve move/rename handling and help independently edited files retain provenance, but filesystem editors can rename or copy files without notifying AVC of intent. An inode or file-content hash alone cannot distinguish rename from copy. First implement deterministic rename/copy inference as **provenance metadata**, not an unconditionally correct canonical file identity. Evaluate explicit native move operations with stable optional identities against ambiguous external filesystem changes before fixing the format.

Store canonical path components without silently normalizing away distinct bytes. Define an explicitly portable naming profile and report unsupported names, symlinks, casefold collisions, reserved paths, and Windows/macOS/Linux differences. Hash algorithm choice (BLAKE3-256 versus SHA-256), domain separation, complete serialization and migration paths require byte-level vectors and threat review; the selected digest is part of the immutable object ID, while Git object IDs remain a distinct namespace.

## 12. Effect on previous design

**Retain:** local-first, immutable snapshots, stable changes, operation-history undo, structured conflicts, bounded private recovery, isolated workspaces, quiet draft synchronization, deliberate validation and publication.

**Replace:** Git/Jujutsu as canonical runtime; two authoritative heads bridged by adapters; Git-host-first semantics driving the local model; a mandatory complex server.

**Re-evaluate by experiment:** flat versus paged directory maps, patch commutation, selective chunking, virtual materialization, distributed caches and new physical compression.

Start implementation with the native transactional snapshot core and rigorous integrity tests, not additional wrappers around the old Git auto-commit implementation.
