# AVC architecture

**Decision date:** 2026-10-13  
**Status:** Accepted implementation direction; not a claim about current behavior  
**Audience:** Implementers of the general-purpose VCS and its integrations

This replaces the August 2026 native-storage-first blueprint. The target is now ordinary existing software projects as well as greenfield projects, with one person or many independent human and automated workers. The current Rust prototype remains a Git command wrapper; [the roadmap](roadmap.md) defines its replacement.

Evidence and alternative-system comparisons belong in [research.md](research.md). This document owns the proposed contracts. Format details and dependency versions must be pinned against implementation evidence rather than inferred from this prose.

## 1. Product decision

Build a **local-first, snapshot-based, change-oriented VCS with isolated workspaces, bounded recovery, Git interoperability, and explicit integration**.

Reuse Jujutsu's history algorithms and Git storage for the first implementation. Build the missing recovery, coordination, API, and integration contracts around that engine. The product is not a new distributed storage engine with a VCS attached, and not an agent scheduler with Git commands attached.

The three ordinary user concepts are **workspace**, **change**, and **target**. Precise revisions, recovery snapshots, operations, proposals, and receipts exist underneath them and are available for inspection. A solo developer should not need a server, credentials, an agent, a policy language, or a workflow object to edit and save a file.

### Decisions that should survive implementation

| Concern | Decision |
| --- | --- |
| Source truth | Immutable snapshots; changes and evolution describe intent, not an imperative replay requirement |
| Human workflow | Editable logical changes, recoverable history operations, optional named branches |
| Concurrent work | Independent mutable workspaces sharing immutable data |
| Recovery | Private, bounded snapshots separate from semantic ancestry and public events |
| Automation | Typed operations with exact inputs, explicit preconditions, stable outcomes, and queryable retries |
| Integration | Validate the candidate actually being integrated, then conditionally advance one target |
| Adoption | Existing Git projects and hosts first; no flag-day forge replacement |
| Implementation | Rust; reuse `jj-lib` and its Git backend; one AVC metadata transaction authority |
| Scale | Incremental local work and checkout-free APIs first; lazy storage and distributed caches when justified |
| Non-goals | A required custom filesystem, source-text CRDT, LLM merge authority, CI engine, sandbox runtime, or full forge |

A future native backend is allowed, not promised. It must preserve these contracts and earn its maintenance cost through representative workloads.

## 2. User workflow

The following is a **proposed interaction**, not a command sequence supported by the current binary:

```text
clone existing Git project
new "Add streaming parser"
edit normally; observe recovery status
split, amend, inspect, or undo as needed
finish the change
propose selected revisions for review
land the accepted candidate into main
```

Finishing a change does not publish it. Saving recovery state does not finish a change. Proposing does not move `main`. Landing does not deploy an application.

A second worker receives a workspace based on a specified immutable revision. Its changes remain separate until explicitly composed. Workspaces may be short-lived or persistent; there is no mandatory branch or remote review for each worker, experiment, or tool call.

The CLI should support ordinary inspection and editing without a required staging area. Partial selection belongs in operations such as split, move, and restore. Familiar aliases are acceptable, but unrelated destructive meanings must not share one command.

## 3. Data model and identity

### Source and intent

A source revision contains an immutable snapshot, ordered parents, authorship, description, and a stable logical change identity. A normal amendment or rebase preserves the change identity and creates a new revision identity. A change can have divergent revisions; there is not necessarily one universal latest revision.

Ancestry and evolution are different relations. Ancestry explains the source state on which work builds. Evolution records replacements, splits, folds, and abandoned alternatives. A split or fold records explicit many-to-many lineage; it does not pretend that one identity simultaneously means several independent changes.

In the Git backend, preserve existing Git objects and IDs rather than re-encoding imported history. Use fully qualified IDs in durable records and APIs, including repository and backend/hash namespace. Short IDs are display and unambiguous interactive lookup conveniences only.

### Recovery snapshots

A recovery snapshot records a workspace's captured tree, base revision, generation, capture time, and capture outcome. It is not automatically another semantic revision or operation ancestor. This separation permits frequent recovery without causing a descendant rewrite or permanent history growth on every edit.

A user operation can promote a captured tree into a revision of the current change. Named checkpoints pin recoverable states. Recovery records and semantic history can share content objects without sharing retention rules.

### Operations and coordination

Jujutsu operations own the source view and its evolution. AVC coordination records own workspace generations and ownership, recovery roots, proposals, validation references, idempotency outcomes, and integration receipts. Do not implement a second competing source-history graph in the coordination database.

A proposal identifies exact source revisions and their dependency closure, a target, its base, and a proposal generation. Review and validation attach to the exact proposal/candidate, not just a change label. Optional task descriptions, agent/model identifiers, and external issue links are metadata; none are required for source reconstruction.

A target has an immutable current revision, a monotonic generation in managed mode, and a policy version. Multiple unrelated targets do not share one logical compare-and-swap condition.

### Hashes and encoding

Own identity-bearing envelopes need versioned, domain-separated canonical encoding and a reviewed digest. Specify exact bytes and test vectors before making a stable format. Do not hash raw JSON presentation, local paths, database pages, timestamps introduced merely by retries, or nondeterministic map iteration.

Preserve imported commit ordering, signatures, and bytes where the operation is identity-preserving. A rewrite necessarily creates new identities and needs new attestations. Content integrity, signer authentication, authorization, and semantic correctness are separate properties.

## 4. One transaction authority

### Local layout and implementation boundary

The initial implementation uses:

- Jujutsu's Git backend for source objects and history algorithms;
- durable immutable Jujutsu operation/view objects;
- a small SQLite database for AVC-owned authoritative metadata and the active source-operation root;
- rebuildable source/query indexes and working-copy caches; and
- ordinary filesystem workspaces.

Use one SQLite database, local storage, a supported SQLite build, WAL mode with an explicitly durable synchronization configuration, and bounded transactions. Do not place the live database on a network filesystem or assume WAL makes filesystem outputs transactional. SQLite documents both its [atomic-commit assumptions](https://www.sqlite.org/atomiccommit.html) and [WAL constraints](https://www.sqlite.org/wal.html).

Source blobs must not be copied into a second application journal merely to update metadata. Conversely, avoiding a metadata WAL is not a justification for inventing storage recovery. The old requirement that everything enter through one custom append-only segment format is removed.

### Source transaction protocol

The adapter prepares immutable objects and an unpublished Jujutsu operation, then publishes their identity with the related coordination update through the AVC authority:

```text
capture declared inputs and workspace generation
prepare immutable source objects and operation/view
make their complete required closure durable and GC-protected
begin short metadata write transaction
  verify expected source/workspace state and request identity
  select the new source-operation root
  update related coordination records
  record the durable request outcome and required outbox events
commit metadata transaction durably
acknowledge the operation
```

The metadata commit is the visibility boundary. Before it, prepared objects may be orphaned and collected later. After it, all referenced objects needed for the operation must be readable and retained. A timeout after commit is resolved by querying the request outcome, not by repeating the side effect blindly.

Jujutsu exposes [unpublished transaction writes](https://docs.rs/jj-lib/latest/jj_lib/transaction/struct.Transaction.html) and an [operation-head store interface](https://docs.rs/jj-lib/latest/jj_lib/op_heads_store/trait.OpHeadsStore.html). These are the intended integration seams, not evidence that the complete adapter already exists. Qualify their exact behavior, durability, GC, and cancellation against a pinned source revision in the first milestone.

The managed repository must not also publish through an independently authoritative default `.jj` operation-head file. Provide the operation-head adapter or load explicit operation IDs through the coordinator. Do not run `jj`, update SQLite afterward, and describe the pair as one transaction. Uncoordinated direct writes to the internal repository are outside managed-mode guarantees; imports must detect and reconcile external Git changes explicitly.

### Concurrency and recovery

Prepare hashing, merges, and queries outside the metadata write transaction. Initially, serialize source-root publication and retry a stale preparation. For independent work, reconcile operation views using the history engine before retrying; never silently overwrite a competing result. Do not hold a repository lock while an agent, editor, merge plugin, or test suite runs.

A single local database still serializes its writes. That is a concrete initial implementation choice, not a claim of unlimited parallel write throughput. Measure contention before adding more writers or a server database.

The coordination database is authoritative, not a disposable index. Backups must include a consistent metadata snapshot and its referenced immutable closure. Deleting query indexes may cost time; deleting the authoritative database is data loss unless restored from a suitable backup/export.

### Filesystem updates are separate

An atomic source-view transaction does not atomically replace an arbitrary working directory. Checkout/restore needs a recoverable materialization intent with old and desired revision IDs, workspace generation, protected original content, and progress. On interruption, resume or preserve both versions for recovery. Never blindly overwrite new user edits while replaying an interrupted checkout.

An operation can report `committed; workspace update pending`. It must not falsely report that all files were updated. Immutable API clients can use the committed revision while filesystem clients wait for successful materialization.

## 5. Working copies and recovery

### Isolation

Each independently writing worker gets a distinct mutable directory and workspace identity. Share immutable object stores, read caches, and safe build caches. Reflinks, sparse materialization, and lazy filesystems are optional accelerators, not correctness requirements.

This is file isolation, not process, network, credential, database, or service isolation. Sandboxing and task scheduling belong to the invoking agent/runtime. Do not give an untrusted worker writable access to the shared repository store or publication credentials and expect a workspace flag to enforce isolation.

A workspace generation and owner/fencing token protect managed API mutations. Administrative takeover increments the generation so a stale worker cannot publish through its old capability. A lease expiry alone does not stop an already running OS process from writing files; revoke its actual access or preserve the abandoned workspace separately before reuse. Generations also do not detect arbitrary editor writes: materialization must compare the actual protected/captured file state before overwriting it.

### Capture guarantees

Watchers, mtimes, editor events, and file-size caches are hints. Overflow or uncertainty causes reconciliation. Capture retries files that change during reading and exposes failures or unstable paths. A normal directory scan cannot promise all files existed simultaneously in the captured combination.

Recovery snapshots may represent such a best-effort capture if clearly labeled. A validation candidate is an immutable tree constructed from specified bytes; tests run against that tree, not against a mutable directory that may change underneath them. Quiescing a worker or a suitable filesystem snapshot can provide a stronger capture boundary, but is not assumed everywhere.

Unwritten editor buffers are not captured by a filesystem watcher. The UI reports the last durable capture, pending or failed work, and capture coverage. Do not promise protection since the last keystroke.

### Admission and privacy

Automatically protect tracked/admitted files. Discover new files, show their protection status, and allow explicit admission or configured safe path policies. Never silently capture ignored files, secrets, build caches, or every untracked file in a repository. A file admission operation is not a Git staging area and does not need to be repeated for each edit.

Ignoring a previously captured secret is not erasure. Provide clear inspection and purge procedures covering local objects, drafts, retained backups, and remote policy. Scanning for likely secrets is useful defense, not a guarantee.

### Retention and garbage collection

Bound automatic recovery by time, count, and storage budget; expose policy and dry-run results. Keep explicit pins separately. Coalesce no-op captures and apply backpressure instead of accumulating unbounded pending work.

Do not attach every savepoint through strong references in an immortal operation chain. Retention boundaries and any compacted operation roots must be explicit. Audit continuity can retain digests without retaining deleted source objects; a digest is not a recovery capability.

GC roots include active source views, retained operations, checkpoints, recovery snapshots, proposals, live validation candidates, accepted remote roots, readers, and in-progress imports. Protect new roots before metadata publication. Release obsolete pins only after a root scan and reader/transaction protection. Qualify aggregate retention pins so frequent saves do not require unbounded public or per-savepoint refs. Do not assume an external forced Git prune honors AVC leases.

Published history has a separate retention policy. User undo and recovery have an advertised retention horizon, not an unlimited promise.

## 6. Concurrent changes and machine operations

### Identity is not a precondition

A mutation of a change must specify the expected revision or proposal generation, not only its stable change ID. A mutation of a workspace must identify that workspace and specify its generation. A landing must specify its expected target state and policy.

Concurrent amendments create explicit divergence or a stale-precondition result. The server must not choose the latest timestamp as the winner. Rewriting dependencies records replacement revisions and invalidates dependent candidates; it must not silently change another active worker's directory.

A worker can continue exploring an older revision. Before proposing or integrating, it must explicitly adopt, rebase onto, or otherwise reconcile the required dependency revisions.

### Checkout-free operations

Expose read tree/file/range, diff, history, change lookup, conflict inspection, create/update revision, compose, validate request, and proposal/integration operations without requiring a working copy. CLI, local library clients, and server clients use the same meanings.

Machine requests carry a schema version, repository identity, request identity, operation, exact inputs, resource limits, and relevant preconditions. Responses contain typed IDs, observed generations, outcomes, and structured recovery information. Immutable pagination is tied to a revision/view; cursors cannot silently drift as `main` changes.

Illustrative envelope, not a frozen wire schema:

```json
{
  "schema": 1,
  "request_epoch": "authority-issued-active-epoch",
  "request_id": "client-generated-unique-id",
  "operation": "change.apply",
  "repository": "repository-id",
  "change": "change-id",
  "expected_revision": "backend:full-revision-id",
  "workspace": "workspace-id",
  "workspace_generation": 12,
  "edits": [
    {
      "path": "src/parser.rs",
      "expected_blob": "backend:full-blob-id",
      "replacement_blob": "backend:new-blob-id"
    }
  ]
}
```

The whole managed source mutation succeeds or fails; a failed precondition does not apply some of the edits. Filesystem materialization remains a separately reported step.

Scope idempotency by authenticated actor, authority-issued request epoch, and request ID. Reusing that identity with a different canonical body is an error. Keep accepted outcomes while their epoch remains active. Retire epochs explicitly and reject requests from retired epochs; do not delete individual outcomes and then mistake a late retry for a new write. The API exposes the retry horizon and an expired outcome. Cancellation is best effort before commit and cannot revoke a committed result. Queries are bounded and stream large output. Machine mode never waits for a terminal prompt.

Local RPC is optional, not required to execute the command. An MCP adapter may expose this API; MCP is not the repository's canonical data model or correctness boundary.

### Stale reasoning and dependencies

Distinguish an explicit change dependency from observed source inputs. Optional read records can include file hashes, relevant directory listings, and negative lookups. They can warn that a worker reasoned from stale inputs even when its final patch applies cleanly.

A declared or partially instrumented read set is not complete. It cannot prove that two changes commute semantically or replace validation. Enforcement requires a controlled observation boundary, and even a complete source read set does not capture unstated intent. The default safe integration policy does not depend on complete read-set tracking.

## 7. History editing and merging

Reuse the history engine for stable changes, ancestry, evolution, conflict-bearing revisions, and reversible source operations. Do not store conflict-marker text as the only conflict truth. Preserve base and alternative terms, deletion/type conflicts, and enough provenance to inspect and resolve them.

Start with deterministic tree and textual merge. Rename/copy information and syntax-aware drivers may improve results, but are not source truth. Optional drivers must be versioned, resource-bounded, deterministic for their declared inputs, and safe on unsupported or invalid syntax. Preserve exact source bytes outside intended edits and retain a conservative fallback. [Mergiraf](https://mergiraf.org/architecture.html) is an implementation reference, not a semantic-correctness oracle.

An LLM resolution is an authored candidate revision requiring ordinary review/validation. It never runs implicitly inside object decoding, fetch, GC, or canonical merge.

Private history can be rewritten. Protected shared history advances normally by new changes or revert, not by silently rewinding a target. An operation that restores source state does not restore expired authority, retract delivered events, undo a deployment, or replay a remote publication. Recovery and external side-effect reversal are deliberately different operations.

Conflict-bearing revisions can be saved and exchanged over a capable native path. Ordinary Git export and landing on a conflict-free target reject unresolved native conflicts explicitly.

## 8. Four collaboration boundaries

| Boundary | Purpose | Default side effects |
| --- | --- | --- |
| Recovery | Protect local edits | No network, review, or CI |
| Draft sync | Back up/share selected private work | Restricted roots and retention; no public project events |
| Propose | Request review of exact selected revisions | Deliberate review update and configured validation |
| Land | Advance shared target state | Required checks and one integration event/receipt |

Remote backup is opt-in and cannot be silently enabled by an imported repository file. Access scope is checked over the complete reachable object closure, not just the named tip. Uploading a public tip whose ancestry includes private drafts can expose those drafts; construct and authorize the exported closure deliberately.

Draft synchronization is not achievable by merely calling a normal Git push less loudly. Standard hosts may generate webhooks, CI, indexing, and notifications for any pushed ref. AVC must expose the negotiated capability: a restricted native draft service can promise quiet synchronization; an ordinary Git remote cannot unless its configured behavior supplies that guarantee.

Upload immutable objects before a conditional root transition. Use bounded bundles or batch fetches, validate hashes/graph closure/resource limits in quarantine, and let retries reuse already accepted objects. Object retrieval does not automatically confer authorization to every object sharing the same digest.

## 9. Integration and validation

The default is **validated integration**, not Scion's post-merge testing policy. A repository owner can choose a less restrictive target, but the resulting guarantee must be labeled accurately.

### Candidate, evidence, and acceptance

An immutable candidate manifest identifies:

- repository, target, expected base revision and target generation;
- ordered selected revisions, dependency closure, and the exact resulting source root/revision;
- merge/rewrite strategy and relevant driver/configuration versions;
- policy digest, required approval subjects, and validation requirements; and
- validation recipe, declared external inputs, and toolchain/environment identity.

The candidate does **not** contain its validation results or approval objects. Evidence refers to the candidate digest, its runner identity/trust, result, and applicable policy. An acceptance record references the candidate and the selected evidence/approval records. A landing receipt references that acceptance and the completed target transition. This ordering avoids circular content identities and keeps a test result from changing the identity of the candidate it tested.

Large logs and outputs live outside source history. Model names or task labels are optional annotations, not proof of who produced bytes or whether they are correct. Review evidence may concern a specific constituent revision; the acceptance policy must explicitly establish how that subject authorizes the composed candidate, especially if rebasing or conflict resolution changed its bytes.

Tests run in an isolated materialization of the immutable candidate. If a command changes source inputs, it creates a new candidate or invalidates the original run; it does not quietly certify the new bytes. Network services, dependencies, and environment inputs must be captured or explicitly treated as non-hermetic.

### Landing protocol

```text
1. Select exact proposed revisions and target base.
2. Compose an immutable candidate; reject unresolved conflicts.
3. Obtain required review and validation evidence; form acceptance.
4. Recheck current authorization and applicable policy.
5. Atomically compare expected target generation/base and policy,
   advance the target, and record receipt plus outbox event.
6. Deliver events at least once with stable event IDs.
```

The checks in steps 4 and 5 must share the authoritative policy/authorization transaction or its equivalent version fence. A policy change or revocation during validation cannot be ignored.

Do not hold target locks during validation. If the target changes, recompute the candidate and revalidate the affected exact inputs. Unchanged path sets alone do not justify reusing a green result. If a build system can prove complete action inputs unchanged, its action cache may be reused; [Bazel's separation of action results and content storage](https://bazel.build/remote/caching) is a useful reference.

A simple counterexample is two changes touching different files: one alters an API contract and the other adds a caller relying on the old contract. Both can apply cleanly and pass their original tests independently. Their integrated candidate still needs applicable combined validation.

### Queues and batches

Use bounded queues, fair admission, superseded-work cancellation, and optional speculative validation to keep up with concurrent workers. Batch independent proposals when policy permits, preserving the selected changes and their lineage rather than requiring an indiscriminate squash.

State whether checks cover every intermediate revision or only the final batch candidate. A passing aggregate does not certify every intermediate commit. Removing a failed member changes the candidate and requires corresponding revalidation; failures need not have a single independent cause.

A linear target requires an integration order. Independent targets can proceed independently, but no architecture removes that ordering requirement for one shared head. The useful metric is validated changes delivered per unit cost and time, not raw speculative commit throughput.

### Git-hosted versus managed authority

Existing Git hosts remain authoritative for their branches and policies. Where available, use their protected-branch and merge-queue mechanisms; GitHub already checks candidates against the target and earlier queue entries in its [merge queue](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/configuring-pull-request-merges/managing-a-merge-queue).

A local database cannot atomically commit a third-party host's branch, policy state, and local receipt. Such adapters use durable intent, conditional host operations, status reconciliation, and explicit unknown outcomes after network failure. They must report the host's actual guarantees and cannot substitute client-side checks for server enforcement.

In a managed service, target state, policy versions, receipt, and outbox are owned by the same authoritative transaction. Git refs served from that service are projections or are updated through its controlled authority, not a second independent writable truth. Never advertise distributed exactly-once execution; provide durable request identity, idempotent transitions, and at-least-once delivery.

## 10. Git interoperability contract

Git interoperability is required for adoption, but is not an assertion that every Git feature is already supported by a dependency. Jujutsu's [compatibility matrix](https://docs.jj-vcs.dev/latest/git-compatibility/) documents gaps that AVC must qualify rather than hide.

| Feature | Required treatment |
| --- | --- |
| Existing commits/trees/blobs and parent order | Preserve unchanged objects and graph identity |
| Git remotes, authentication, branches, ordinary review/CI | First usable release's collaboration path |
| Change IDs, evolution, recovery, native conflicts | Preserve natively; declare what ordinary Git exchange cannot carry |
| Arbitrary Git amend/rebase/squash | Import as external history; never assume it preserved logical identities or approvals |
| Signed commits/tags | Preserve existing signed bytes; rewrites need new signatures; qualify creation separately |
| `.gitignore`, executable bits, symlinks, paths | Test the supported semantics and platform limitations |
| `.gitattributes`, EOL conversion, filters, hooks | Detect requirements; implement or explicitly block affected operations; never silently corrupt content or imply checks ran |
| Submodules and LFS | Preserve opaque references; do not claim usable checkout/edit support before end-to-end qualification |
| Shallow/partial repositories | Explicit completeness and hydration behavior; reject unsupported mutations |
| Mixed Git/AVC writes and colocation | Explicit ownership/import rules; no two writers racing the same mutable worktree/index |
| Quiet sync, atomic receipts, custom policy | Capability-dependent; not guaranteed by generic Git push |

The broad product should eventually serve projects using common attributes, submodules, large assets, and signing. A narrow first release must name its exclusions. Delegating unsupported content conversion to a controlled Git adapter can be better than reproducing Git behavior, but subprocess execution, filters, and hooks are effects requiring appropriate trust.

Do not change the user's index or public branch for every automatic recovery snapshot. Private roots must not appear in default export/push refspecs. An administrator deliberately mirroring internal refs can still disclose private history; internal-store access is privileged, not a privacy sandbox.

Full repositories are the offline default. Optional lazy clones explicitly identify missing promised objects and allow users to pin a working set for offline use. Missing promised content is not a deleted file. An unavailable network must not silently produce an incomplete diff, merge, or checkout. Git's [partial-clone design](https://git-scm.com/docs/partial-clone) is relevant precedent.

## 11. Scale without imposing a platform

The deployment path is local tool, existing Git collaboration, optional small managed service, then scale features selected by evidence. No deployment requires all four.

All reads accept immutable roots and explicit scope. Support batched object requests, changed-subtree pruning, bounded graph traversal, stable pagination, and deterministic derived indexes. A query may be served locally or remotely without changing its meaning. Index cache keys include algorithm and configuration versions; invalidation must not change source truth.

Keep object count, changed-byte volume, file count, history depth, workspace count, ref count, query fanout, and CI/event amplification as separate performance dimensions. Sharing snapshots does not mean copying every file per revision. Conversely, Git-style flat directory objects can still make very large directories expensive.

Initially use the existing Git backend's storage and maintenance mechanisms. Benchmark tuned Git and Jujutsu before adopting custom packs, paged Merkle maps, changed-file canonical logs, content-defined chunking, or new transport reconciliation. A native representation must demonstrate improvements after including import/export, query cost, recovery, and maintenance.

Scion and Sapling motivate checkout-free APIs, lazy materialization, and independent query/storage/cache layers, not mandatory regional infrastructure for a five-person project. Large deployments can add cache tiers with common APIs, backpressure, and failure isolation. Avoid a network round trip per file or a single global repository-view CAS for unrelated target updates.

## 12. Large files, artifacts, and filesystem boundaries

Versioned assets are legitimate source inputs. Support ordinary binary snapshots and explicit binary conflicts; later qualify LFS or content-chunked storage without changing logical file identity because of physical chunk placement. Do not put build outputs, transient logs, model traces, or caches into source history by default.

Some assets are not usefully mergeable. Provide optional server-enforced exclusive publication leases with fencing for selected paths, inspired by [Perforce's exclusive-open model](https://help.perforce.com/helix-core/server-apps/cmdref/2026.1/Content/P4Guide/resolve.lock.exclusive.html). Offline edits may continue as drafts, but cannot promise global exclusivity. Ordinary source text remains optimistically concurrent.

Materialization must reject traversal, unsafe symlink following, path aliases/case collisions, invalid reserved paths, and unsupported file types rather than silently overwrite different content. Preserve the original source representation when a platform cannot materialize it; return an explicit portability error. The first engine's UTF-8/path constraints must be exposed, not confused with arbitrary Git path support.

Artifact services have independent retention, quotas, access control, and GC. A source revision may reference an artifact or attestation digest without retaining the entire build cache. Test results are evidence under a stated policy, not proof of program correctness.

## 13. Trust and extensibility

Authenticate the actor and authorize repository, operation, path scope, target, and data visibility at the boundary that enforces them. Separate write-draft, propose, attest, and land authority. Revocation and policy epochs are checked again at commit. Client-provided agent names and local advisory locks are not security principals.

Repository configuration is untrusted input until approved. Clone, status, diff, fetch, and GC must not implicitly execute repository-supplied hooks, plugins, package scripts, or LLM calls. Test/build execution belongs to a deliberate runner with its own sandbox and credentials.

Do not save raw prompts, credentials, or full agent transcripts by default. Optional provenance should identify relevant artifacts and decisions without turning source control into permanent surveillance. Hashes do not confer confidentiality or authorization; avoid cross-tenant object-existence disclosure and unrestricted global deduplication.

Use a small versioned library/API boundary rather than an in-process arbitrary plugin ecosystem. Orchestrators can implement agents, reviews, CI, sandboxing, and deployment without being dependencies of the core repository.

## 14. Correctness obligations

The implementation and failure tests must establish these observable properties:

1. Acknowledged source mutations have a durable complete required object closure and a queryable outcome.
2. A stale source, workspace, target, or policy precondition cannot silently overwrite current state.
3. Concurrent workers cannot accidentally share mutable worktree state through the default API.
4. Interrupted materialization preserves recoverable content and never masquerades as a completed checkout.
5. Recovery retention actually bounds retained data without deleting live, pinned, or leased roots.
6. Approvals and validation are attached to the exact subject and applicable policy, not just a stable change name.
7. A target cannot advance under expired authority or evidence for a different candidate.
8. Private capture and native draft sync do not implicitly become public review/CI events.
9. Deleting derived indexes changes performance, not the answer; restoring authoritative state requires its real backup.
10. Ordinary Git exchange is lossless for its declared supported subset and explicit about unsupported metadata or content behavior.
11. Retries, cancellation, timeouts, and duplicate events do not silently duplicate integration.
12. Textual or structural merge success is never represented as proof of semantic compatibility.

These are contracts to implement, not guarantees inherited automatically from Git, Jujutsu, SQLite, a hash, or an API name.

## 15. What changed from the previous design

The snapshot model, stable changes, recoverable history, structured conflicts, quiet drafts, and deliberate publication remain because they serve the requirements. The new design makes five substantive changes:

- **Git adoption and engine reuse move to the beginning.** A new canonical storage format and new forge are no longer prerequisites.
- **Recovery snapshots are detached from semantic revision churn.** Frequent saves must not force graph rewrites or immortal ancestry.
- **Concurrency is a first-class contract.** Isolated workspaces, exact preconditions, explicit divergence, and protected handoff replace assumptions of one interactive writer.
- **Integration binds source, evidence, policy, and authority.** A generic publication manifest alone was insufficient.
- **Authority and capability boundaries are explicit.** Metadata versus source objects, repository transactions versus checkout, local state versus third-party hosts, and file isolation versus sandboxing are not conflated.

The first implementation should prove the integrated product on existing repositories. The decision to own more of the history or storage engine must follow an identified contract mismatch or measured bottleneck, not a desire to make the project appear more novel.
