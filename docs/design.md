# AVC: concrete reference design and decision boundaries

**Reviewed:** 2026-10-14.  
**Status:** Candidate A for implementation and comparison; not a frozen source format or a validated production system.  
**Evidence:** [research.md](research.md). **Qualification:** [roadmap.md](roadmap.md).

The product is free to be native. Git and Jujutsu are not required as its storage or history engine. The previous selection of State/WorkVersion/Application as a novel fundamental model is superseded: it described provenance but did not demonstrate a better merge algorithm. A causal change graph and a delta-first changeset store remain complete competing candidates described in the research record.

## 1. Objective and why this candidate leads

The target is ordinary software projects, from a solo developer to teams running many agents, with an optional path to large hosted repositories. Optimize inspect/edit/recover/compare/integrate tasks, including failed attempts. Require exact bytes, understandable history, offline work on retained data, clear conflicts, safe publication and explicit interoperability.

Candidate A uses immutable source-tree revisions, explicit change evolution, isolated workspaces, transactional resource updates and checkout-free APIs. This is selected as the **reference design** because it handles arbitrary filesystem-produced source without requiring a parser or an exact event stream from every editor. It exposes the source tested by CI directly and provides a clear Git-history projection.

Those reasons do not prove that snapshots are inherently faster or safer than a well-indexed change graph. Candidate B may win where causal selective undo and repeated reapplication dominate. Candidate C may win where admitting many rarely read draft versions dominates. Do not silently promote this reference choice to a stability commitment before the comparisons in the roadmap.

## 2. User model

A person works in a **workspace**, groups work into **changes**, and deliberately updates a **shared target**. Internal revisions and recovery records are inspectable without requiring a user to manage all of them.

Proposed operations, not commands supported by today's prototype:

```text
new "Add parser"          begin a logical change
capture                   preserve admitted filesystem bytes privately
finish                    create a described source revision; no publication
workspace create <base>   give another worker an independent directory
merge <revision>          create an integration result or typed conflicts
restore <revision> --to <new-directory>
propose <revisions>       create/update an exact review subject
land <candidate>          request policy-controlled target advancement
```

Automated capture is opt-in per workspace or established repository policy, visible, bounded and independent of review. New files are discovered but not all silently retained. Alternative experiments and compositional tasks both use workspaces; deleting other workers after one succeeds is not an implicit VCS policy.

The VCS does not orchestrate agents, run an unrequested build, create a sandbox, deploy software, or infer that every tool call deserves a commit. A server is unnecessary for local operations.

## 3. Authoritative records

These are logical schemas. They define interpretation, not a stable binary layout:

```text
Tree = ordered map<PathComponent, Entry>
Entry = File(content_id, executable) | Symlink(target_bytes)
      | Directory(tree_id) | Conflict(conflict_id)

Revision = {
  format, tree_id, ordered_parent_ids,
  change_id, author, description, authored_metadata
}
Evolution = {kind, predecessor_revision_ids, successor_revision_ids}
Conflict = {kind, base_terms, ordered_side_terms, source_revision_ids, driver_id}

Resource = {kind, resource_id, generation, value}
WorkspaceValue = {revision_id, materialization_state, ownership_generation}
TargetValue = {revision_id, policy_version}
Capture = {workspace_id, tree_id, observed_generation, coverage, expiry}
Operation = {request_identity, affected_resources, before_values, after_values}
Candidate = {base_revision, expected_target_generation, selected_revisions,
             result_revision, recipe_id, validation_policy_version}
Evidence = {candidate_id, validation_inputs, runner_identity, result}
```

A full schema must distinguish source author time from capture/receipt time; neither is a concurrency ordering mechanism. Change IDs are stable labels for intent, not a magical transformation law. Revision IDs identify immutable bytes and metadata. Amend/rebase may create new revisions with the same change ID; concurrent successors remain distinct. Splits and folds use explicit many-to-many evolution records. No timestamp chooses a globally latest variant.

The source tree identifies source bytes and relevant file kinds/modes; it does not by itself identify causal history or which independent edits contributed identical text. Parents, evolution and merge provenance preserve history separately. Do not promise causal selective undo solely because that metadata exists: the transformation law still has to be specified.

### Object identity

Use a domain-separated hash of object kind, logical format version and canonical payload. A reference implementation may use SHA-256 and publish byte-level vectors; this does not decide a final hash/performance policy. Short hashes are display-only. Maps, parent ordering, lengths and metadata bounds need canonical rules.

Changing compression, segment location or physical delta packing must not change IDs. Changing a canonical directory's paging layout or chunk-manifest encoding **can** change its ID. Equal logical content yields equal IDs within the specified format; do not assert representation-independent roots without a matching commitment scheme. Keep native IDs and imported Git object IDs in distinct namespaces.

The first tree representation is hierarchical sorted entries, with whole-file blobs. Empty directories, executable bits and symlinks have explicit semantics. Devices/sockets and unsupported metadata are rejected. Canonical filename bytes and each platform's materializable subset are different; refuse case/normalization/reserved-name collisions rather than silently alter spelling.

## 4. Source operations and merge algorithm

### Direct edits

`apply_edits(base_revision, edits, expected_resource_generation)` creates a new source tree from immutable inputs. Each edit identifies its path and expected previous entry/content ID, including absence. Validate the entire edit set against the base before publishing any result. A stale generation or entry returns a typed conflict, not partial success or implicit fuzzy patching.

Source updates through this API are atomic at the repository level. Updating ordinary filesystem files is a separate operation described below.

### Integration

`merge(base, left, right, driver_policy)` begins with a deterministic tree comparison. For each entry, the reference rule is:

```text
left == right : select those bytes, retain both source identities in provenance
left == base  : select right
right == base : select left
both directories : recurse
supported text : run the explicitly selected, versioned merge driver
otherwise : retain a typed conflict with base and alternatives
```

Directory/file collisions, delete/modify, modes, symlinks and binary replacements are entry-level conflicts unless a specific safe rule applies. Renames are explicit hints or deterministic inference, never unquestionable identities extracted from an inode or matching content. Copy and move are different relations. Unknown or malformed syntax remains valid source data; a parser may decline and leave a conservative conflict.

For multiple merge bases, calculate an explicit virtual base using the same conflict-preserving rules and record it. Do not quietly choose an arbitrary ancestor. For more than two selected revisions, use an explicit integration order/plan; there is no claim that arbitrary merges are associative or that checking all possible orders is cheap. Text, token and structural drivers are replaceable components with separately measured correctness.

The result is a new revision with parent/provenance references to its inputs. Existing authored revisions remain immutable, as they can in existing VCSs. A separate Application object is not necessary merely to restate that fact. An operation or candidate records an exact merge recipe where useful.

Conflicts are records, not only marker text. Preserve original alternatives and any remaining merge terms. A resolution is a new source revision tied to its actual conflict inputs. Reuse a previous resolution only under a declared matching context; otherwise propose it for inspection or keep the conflict. A clean result is not proof of program compatibility.

### Same-change policy and undo

The reference user policy coalesces independently identical changes in the rendered file while retaining source provenance. Jujutsu documents a similar usability-versus-algebra tradeoff; see the research record. We do not promise that reverting A retracts only A's causal effect while preserving an independently identical effect B.

`undo` restores prior local resource values under current preconditions and retention limits. `revert` creates an inverse-content change on current source, possibly conflicting. **Causal selective undo** is a distinct challenger requirement. It must name the intended behavior for repeated identical deletes, later edits, dependencies and delete/recreate before being implemented or advertised.

A target that integrated a change, reverted it, and later intentionally reapplies it must remain supported. An eternal set of ever-seen ChangeIds is not a valid duplicate filter. Deduplicate retries by request identity; distinguish ancestry merge, explicit cherry-pick/reapply and change evolution.

## 5. Independent roots and one transaction boundary

Mutable resource keys include workspaces, local names, proposals, retention pins and shared targets. Each has a generation. A source mutation checks only the resources it actually changes; a multi-resource operation checks them together. A single local writer may serialize short physical transactions without causing false logical conflicts between unrelated workspaces.

Prepare hashing, diff, merge and candidate computation outside the write transaction. Do not hold it while a model, editor, build or remote service runs. The source operation publishes its objects and resource transition together, then returns an immutable result ID.

For the reference implementation, use **one local SQLite database for small immutable objects, resource roots, operation records, request outcomes and event intents**, with explicit durable configuration and size/resource bounds. This minimizes cross-file authority during correctness work. It is not a measured storage winner or a mandate to put large assets in SQLite indefinitely. Keep the repository logic separate from physical storage without inventing a broad plugin framework.

Conceptual transaction:

```text
prepare canonical objects and requested result
BEGIN IMMEDIATE
  verify actor/request epoch and canonical request digest
  return a recorded duplicate outcome, or reject conflicting identity reuse
  check all affected resource generations and current policy when applicable
  insert missing immutable objects; verify complete required closure
  update affected roots and their generations
  record before/after operation, retention pins, result and needed outbox events
COMMIT with the selected durable guarantees
acknowledge result
```

WAL mode with synchronous=FULL is a reference configuration, subject to the actual SQLite build/filesystem assumptions. Normal process termination tests do not establish power-loss safety. Long-running readers/checkpoints, transaction size and single-writer contention require measurement. A database is authoritative here, not a cache that may simply be deleted.

A larger-store design can put source payloads in immutable segments. Then the protocol must durably register/protect staging data, flush objects and required directory/manifests, and only afterward publish roots. GC must observe staging references or equivalent generation barriers. The toy probe reproduces how root CAS alone fails. A single append-only object/root journal is another challenger; its recovery and compaction need the same observable contracts.

Backups include a consistent authority snapshot and required object closure. Physical compaction must wait for reader/in-flight protection before retiring old data. Root logs and operation records require actual pruning rules; calling something append-only does not solve retention.

## 6. Working copies, capture and restoration

Each independent writer gets a separate mutable workspace. Immutable content and safe read caches may be shared; mutable hard links must not cause one workspace's writes to alter another. Reflinks, sparse materialization or a virtual filesystem are accelerators, not correctness assumptions.

An actor can own several sessions and workspaces. Generation/fencing tokens restrict managed API writes, not arbitrary OS writes by a process holding access. A workspace is not a credential/network/process sandbox. Administrative takeover cannot revoke a stale file descriptor merely by changing a database row.

Capture admits tracked/explicitly included paths, detects changed bytes and builds a recoverable source state. Watcher notifications and stat caches are hints; overflow or uncertainty causes reconciliation. Files can change across or during reads, and unsaved buffers are invisible. Report exact capture coverage and the last durable result. Validation runs on a separately materialized immutable source root rather than an assumed simultaneous directory scan.

**Restoration has two explicit contracts:**

- In an ordinary directory with uncontrolled editors, a pre-write check detects many stale states but cannot guarantee no-clobber. The reference safe recovery operation materializes into a **new directory** without replacing current files. In-place switches require a cooperative/quiescent ownership contract, explicit dirtiness checks and preserved before-images; limitations remain visible.
- In a controlled workspace where every writer is excluded or fenced at the actual filesystem boundary, record a durable materialization intent, preserve required original data, stage replacements and finish them under that exclusion. On interruption, resume or preserve both versions; report pending materialization separately from committed source state.

Do not claim a universal atomic compare-content-and-replace operation on ordinary portable filesystems. Do not silently weaken this contract merely to make `restore` look simpler.

## 7. Recovery retention and privacy

A successful automatic capture means its bytes are durable within a documented retained horizon. It does not mean review, remote backup, or publication. Show those states separately. Do not permanently link every capture through strong immutable parents; otherwise time-based pruning cannot reclaim source objects.

The initial capture implementation may store a tree root directly. A bounded delta/overlay chain can be introduced only with a maximum reconstruction path, pinned bases, verified effects and measured benefit. An overlay is not an independent byte authority allowed to replay over newer editor writes.

Retention has separate classes: expiring private captures, explicit checkpoints, draft/proposal roots, accepted history, and in-flight/reader protection. Storage pressure cannot silently evict a named pin or an accepted revision. Ignoring a captured secret is not erasure; purge behavior must cover backups and remotes. Build outputs, test logs and model transcripts have separate storage and retention by default.

## 8. Machine interface and distributed use

All source reads and candidate-producing writes operate on exact immutable IDs. Core methods include read/list/range, diff, history, ancestry, apply_edits, merge, resolve, capture registration, propose and target advancement. CLI and remote methods share semantics, not necessarily the same physical execution path. Machine mode has bounded output, stable cursors bound to a revision, structured errors, cancellation and no terminal prompts.

Requests carry schema, repository, actor scope, authority-issued request epoch, unique request ID, canonical request digest and relevant expected generations. Preserve outcomes while an epoch is active. Retire epochs explicitly; a delayed retry from an expired epoch cannot become an apparently new write. Lost responses produce queryable outcomes, not distributed exactly-once promises.

Full local stores work offline. Partial clients explicitly know which objects are promised and can pin a working set. Missing remote content fails or hydrates; it is never an empty file or a silently omitted ancestor. Native transport batches requested objects and queries, verifies hashes/closure, supports resume and authorization, and permits optional node/regional caches. A small local project does not need that infrastructure.

For a hosted service, immutable objects can live in durable blob storage while stateless compute and derived indexes scale independently. Keep coordination around named target updates small. Avoid imposing local pack maintenance on every job, but compare network bytes and cold-cache compute before abandoning useful physical delta compression.

## 9. Review, validation and publication

Capture, restricted draft sync, proposal update and target publication are separate event classes. Ordinary Git pushes may not provide quiet draft semantics; expose that capability instead of silently claiming it.

A Candidate names its exact result revision, expected target/base, selected inputs, integration recipe and policy version. Evidence references the Candidate and its actual toolchain, test recipe, external inputs and runner identity. Candidate identity does not contain its future evidence; that would be circular. A build action may safely reuse output only when its declared full inputs match, not because two changes touch different paths.

Validation never holds the target lock. For a protected target, atomically check its current generation, applicable policy and live authority, then update it and record receipt/outbox. If the base changed, construct and validate the new relevant candidate. Recheck revoked authority and altered policy rather than trusting an old approval forever. Delivery is at least once with stable event IDs.

A serial validated batch containing k changes and requiring T time has an approximate k/T capacity limit before failures/overheads. Support bounded queues, grouping, superseded-work cancellation and sound caching. Alternatively a repository owner may separate accepted and verified targets or choose post-merge/pre-deploy validation. These have different guarantees; none makes arbitrary concurrent changes semantically independent.

Git-hosted targets remain under their host's authoritative policy and conditional writes. A local transaction cannot atomically own the remote branch, review database and local receipt. Adapters reconcile durable intent and uncertain network results, using actual host protections/merge queues where available.

## 10. Interoperability and safety boundary

Git is an import/export boundary, not the core. Preserve unchanged imported Git objects or retain mappings to exact originals. Native rewriting usually creates different Git objects; signatures, change evolution and native conflicts may not translate. Test byte content, modes, parent ordering, tags/signatures, attributes/filters/EOL, ignores, path encodings, LFS, submodules and shallow/partial histories. Refuse unsupported mutation before damage. Initial native workspaces must not race a Git writer on the same index/directory.

A content hash is not authorization, a signature is not a test, and neither provides transport confidentiality. Authorize object retrieval and reachable closure; avoid cross-tenant existence leaks. Repository configuration and fetched content do not implicitly execute hooks/plugins. Process sandboxing belongs to the caller/runner with explicit authority.

## 11. Complexity targets and things not promised

Tree equality is a digest comparison within a logical format. Path lookup follows directory nodes. A snapshot edit rewrites changed content and ancestors; a very wide flat directory can still cost its full width. Tree diff can skip equal subtrees, but worst-case merge and ancestry queries can be large. Indexes need explicit versioning and rebuild cost.

Clean status can be cheap under a valid observation boundary; arbitrary watcher loss or uncoordinated filesystem mutations may require scanning. Metadata-only workspace creation can be constant work while materializing its requested files is not. No tree design, CRDT, stable change ID or CI policy makes all these operations constant time.

The reference design deliberately does not promise patch commutation, causal selective undo, perfectly inferred rename identity, free old-client compatibility after causal pruning, or semantic correctness from merge success. Those require different contracts and evidence.

## 12. What would replace this candidate?

Choose causal graph B if controlled comparisons establish substantially better conflict preservation/selective undo/reapplication with acceptable generic editing, cold reads, retention and interoperability. Choose delta-first C if measured draft admission savings exceed deferred materialization and recovery costs for the intended population. Choose an existing history engine if the native model provides no material user benefit; the freedom to design natively is not a requirement to ignore that outcome.

Until then, this document supplies an implementable reference, not another unsupported claim that its labels or components make it state of the art. The mandatory commitments are the user-facing and integrity contracts; source representation and physical formats remain reviewable before stabilization.
