# AVC implementation and verification roadmap

**Updated 2026-10-14.** [Design](design.md) is the coherent native reference; [research](research.md) explains the alternatives and evidentiary limits. Implementation remains Rust 0.0.1's old Git wrapper.

The immediate goal is **a trustworthy, usable local VCS vertical slice**, not a distributed service and not three full competing products. Build the leading **parent-relative changeset** reference, then compare its costly decisions with a snapshot-root alternative using the *same* storage, merging and workload fixtures. A large-scale client/server deployment is not a prerequisite for the first working tool.

## 0. Establish the invariant and format oracle

Write a small independent source-state model and precise canonical encodings for a root changeset, first-parent Set/Delete, multiple parents, file types, conflict objects, stable logical ChangeIds, resource generations and request outcomes.

Hard cases for the reference interpreter:

- A root revision plus empty/absent parent, Set/Delete repeat and path type collisions.
- A two-parent merge whose stored first-parent delta fully reconstructs the desired result; other parents do not silently change source bytes.
- Same file bytes reached by different histories; equal derived source trees with different changeset identities.
- Mixed file modes, symlinks, opaque blobs and invalid/portable paths.
- Unresolved conflicts and resolutions with exact provenance.
- Concurrent amendments/rebases retaining stable ChangeId while immutable revision IDs change.
- Workspace A advances while B advances independently; shared target CAS rejects a stale actor.
- Lost reply after committed transaction; repeated request produces original outcome.
- Derived source-tree cache deleted: rederivation reconstructs identical logical bytes.
- Root transaction aborted: partially created objects remain invisible.

Publish format hash vectors and reference-model fixtures before declaring a stable storage format. Internal structure can evolve without a compatibility shim while unreleased.

## 1. Working native local loop

Replace the old Git auto-commit/reset command paths as functionality becomes real; do not preserve a permanent second backend or attribute native atomicity to Git+JSON dual writes.

Implement one small Rust crate/library with cohesive boundaries: source/change objects; derived tree/index; repository transaction authority; ordinary working-copy observer/materializer; CLI.

Suggested user-level capabilities, spelling not fixed:

    init
    status
    checkpoint             // explicit protected source capture
    change new / finish    // create one authored native changeset
    log / diff / show
    restore <id> --into <new-directory>

The reference store uses SQLite for small canonical objects and resource roots in one explicit durability domain. Store file bytes exactly; treat unsupported oversized content explicitly until a tested external-blob staging protocol exists. Source-object IDs do not depend on SQLite page layout.

Source queries must work directly against revisions **without a checkout**. Derive a small sorted hierarchical source-tree view immediately as needed, cache by changeset/derivation version, and demonstrate rebuilding it after deleting every derived cache. Queries must not serve partial state when derivation lags.

Acceptance: actual repository reopen, byte-identical files/modes, correct first-parent merge interpretation, deterministic derivation, no missing objects after an acknowledged transaction, separate restore destination, clear unsupported-state errors. Run normal Rust tests, formatting/clippy and focused reference-model probes.

## 2. Focused A versus C comparison before format freeze

Build the **smallest real snapshot-root alternative** over the same Rust object store, persistent directory map, fixture corpus and mutation interface. Do not build two independent CLIs, network stacks or working-copy engines.

Measure:

| Workload | Measurements that decide |
| --- | --- |
| Repeated one-file edits | Canonical write bytes, foreground p50/p95/p99, fsync time, derived tree creation |
| Many small files | Local revisions, status/diff and storage versus tuned Git/Jujutsu |
| Frequently consumed revisions | First-read latency, cumulative write+read CPU/IO and source-tree retention |
| Mostly discarded drafts | Admission cost including later GC and unused derived work |
| 100k revisions, missing derived cache | Cold ancestor reconstruction, index rebuild, peak RAM and time |
| Large directory rename/copy | Number of canonical path edits versus subtree reuse; source conversion cost |
| 1, 8, 32 independent workspaces | Unnecessary logical conflicts, write serialization and aggregate work |
| Binary and missing-object cases | Whole-blob cost, cold transfer, corruption detection, explicit errors |

For both models, use the **same** durability level, changed-file discovery, source blobs, tree encoding, merge implementation and hardware/filesystem. Compare complete creation + first read + merge + collection, not just append. Set actual budgets only from measured baselines.

**Selection rule:** keep C if deferring/indexing source trees yields a meaningful end-to-end gain without unacceptable cold-read, repair, move or ordinary-user regressions. Select A if its immediate tree access and simpler invariant consistently win. The output is one agreed canonical source format and a recorded falsifiable decision, not perpetual research.

A causal patch graph is a separate comparison **only** if causal selective undo/reordering yields demonstrated user value that A/C cannot provide. Reference Pijul/Darcs semantics faithfully; a simplified file-diff algebra is not an adequate substitute.

## 3. Local correctness and user-facing history

Implement:

- Independent workspace roots and generations; safe allocation/reuse of directories.
- A separate append/recover history of resource operations, with meaningful undo versus content revert and explicit reapplication.
- Private bounded automatic capture of tracked/admitted paths, named pins and honest last-durable coverage. No public push per capture.
- Typed merge conflicts; deterministic three-way and proven multi-base handling; explicit path/mode/delete cases.
- Indexes for source ancestry and path-history queries, derived and rebuildable.
- Machine API using exact IDs, expected generations, limits, idempotency epochs, and structured stale/conflict outcomes.
- Safe materialization: fresh-directory restore by default; controlled in-place switches report limited guarantees rather than promising atomic file replacement under arbitrary editors.

Run fault injection around object commit, root selection, journal recovery, retention, in-flight staging versus GC, long readers, interrupted checkout and duplicates. Match supported OS/filesystem semantics; process termination tests alone do not establish power-loss durability. Keep independent backups.

Keep [contract probes](experiments/vcs_contract_probes.py) and [recorded results](experiments/vcs_contract_results.json) as narrow regression reference cases. They are **not** production VCS benchmarks or user-trust certification.

## 4. Git import/export and independent collaboration

Make a real user migration path before spending time on a native distributed forge:

- Import Git file and commit history into native changesets, preserving original Git IDs/objects where truly unchanged and mapping rewritten identity.
- Export selected **resolved** native changesets and accepted target history into a valid Git repository for current hosting/review.
- Explicitly qualify names, modes, symlinks, attributes/EOL/filters, signed commits/tags, LFS, submodules, shallow/partial histories and binary files. Unsupported cases reject before damage.
- Do not let native and Git tools simultaneously write one mutable worktree/index or treat Git remote target policy as local authority.
- Compare an ordinary solo workflow and concurrent agent tasks with maintained Git, current Jujutsu and (where relevant) Sapling. Include failed attempts, remote CI/event counts and operator cost.

If AVC offers only marginal improvements over existing systems on complete tasks, rethink the product scope or the canonical model. Novelty is not a release justification.

## 5. Optional hosted service, not a required architecture

Only after a safe local system works:

- Immutable root/object transfer with batched, resumable sparse requests; locally cached or service-computed diffs, ancestry and merges.
- Restricted draft roots for opt-in backup without creating public review/CI noise.
- Managed target CAS under actual authorization/policy, with receipt and event outbox, exact candidate validation, queryable retries, and separate accepted/verified policies when explicitly chosen.
- Cache tiers with authorization and isolation; failures or misses must not silently return incomplete source.
- Full durability, GC staging, quotas, retention, redaction and backup/restore across service components.

For a single target, validated integration has an inherent ordering/compute bottleneck; use honest queues/batching and precise cache reuse rather than claiming that unlimited agent writers imply unlimited safe merges.

## 6. Performance and trust gates for every phase

Track full lifecycle: create, first materialization, status, diff/history/blame, merge, recovery, sync, index rebuild, GC, Git exchange, and integration. Include small projects, wide trees, substantial histories, binary edits, sparse clients, 100k changesets, up to 1m bounded private captures and 1/8/32 workers. Record source/tool versions, reproducible datasets, warm/cold caches, filesystem, hardware, p50/p95/p99 and raw results.

Absolute requirements: no acknowledged missing content, no silent overwriting of external filesystem edits, no unauthorized disclosure/target updates, no lost unexpired recovery pins, no wrong-source validation receipts, and explicit interoperability failures. Measure both **unnecessary merge conflicts** and **incorrect automatic merges**.

The current crate's test commands remain:

    cargo test
    cargo fmt --check
    cargo clippy --all-targets --all-features -- -D warnings

Do not bump the version, rename AVC, or claim native code has been implemented in this design/documentation change.
