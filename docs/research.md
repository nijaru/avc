# Native VCS decision: evidence and tradeoffs

**Reviewed:** October 13, 2026. **Decision:** native authoritative VCS; Git/Jujutsu as comparators and interoperability references, not the core.

[Architecture](design.md) owns the selected design. [Roadmap](roadmap.md) owns proof and implementation. This supersedes the earlier selection of Jujutsu's history engine plus Git's object backend.

## 1. Reason for changing direction

A slightly nicer Git/Jujutsu experience is unlikely to justify moving important work to another VCS. The stronger opportunity is a coherent model for recoverable local history, agent workspaces, high-rate incremental operations, stable change identities, typed conditional APIs and verified integration.

The absence of a hard development deadline lets us choose architecture for long-term value. But implementation time is not the only cost: an unproven native storage engine has **more independent correctness and compatibility failure modes** than reusing mature engines. A layered engine also introduces cross-boundary transaction, garbage-collection and state-owner hazards. The risks are different; they cannot be asserted equal without evidence. Users' data loss and adoption friction must govern release readiness.

There is no evidence that AVC currently outperforms existing systems, nor that a new format will automatically do so.

## 2. Alternatives

| Alternative | Strength | Constraint | Verdict |
| --- | --- | --- | --- |
| Git plus an agent-friendly CLI | Maximum compatibility and proven object storage | Most unique semantics live in glue layers; limited differentiation | Baseline, not selected |
| Jujutsu engine + Git objects | Tested history editing, operations and Git interchange | Some extra state/identity mapped around Git, ownership seams and inherited model | Strong alternative, not selected |
| Jujutsu model + native backend | Pluggable storage; reduced history-engine work | Still adopts jj semantics/API contracts; could be sufficient if our model converges | Reconsider if evidence supports |
| **Native snapshot/change VCS** | Full control of objects, source identity, transaction, recovery, sync and interface | Own correctness, migration, implementation and operational burden | **Selected** |
| Native patch-algebra VCS | Strong theory of independent reorderable changes | Universal binary/path semantics, proof and query/interop burdens | Research branch |
| Scion-style always-online changeset service | Extreme centralized throughput and query scale | Not ideal for ordinary offline development | Scale reference only |

If native workflow results turn out marginal compared with tuned Git or Jujutsu, rethink differentiation; do not keep novelty for its own sake.

## 3. Verified prior art and lessons

**Git.** Its [immutable trees and blobs](https://git-scm.com/book/en/v2/Git-Internals-Git-Objects) already share unchanged content; commits are not full physical repository copies. [Worktrees](https://git-scm.com/docs/git-worktree), [partial clone](https://git-scm.com/docs/partial-clone) and [commit graph](https://git-scm.com/docs/gitformat-commit-graph) address real scaling problems. A native system must beat a *tuned* Git baseline, not a straw man. Hashes/dedup alone are not differentiation.

**Jujutsu.** Its [architecture](https://docs.jj-vcs.dev/latest/technical/architecture/) documents separate commit/operation/head/index backends, including a proof-of-concept SimpleBackend, and stable-change/operation models. [Concurrency](https://docs.jj-vcs.dev/latest/technical/concurrency/) and [conflicts](https://docs.jj-vcs.dev/latest/conflicts/) already solve important human workflows. Jujutsu can use native storage without Git; a native object format alone is therefore not reason enough to replace it. The [Git compatibility matrix](https://docs.jj-vcs.dev/latest/git-compatibility/) also notes limitations (such as attributes, submodules and LFS). Our advantage must be in end-to-end contracts, not claiming jj lacks those features.

**Anthropic Scion.** Its [October 13, 2026 design](https://www.anthropic.com/engineering/redesigning-source-control) uses canonical changed-file log entries, recursively computed derived indexes, server-side operations, no required local full repo, and tiered caches. Anthropic reports over 30,000 merged changes/day, roughly 100x faster checkouts and 200x less CPU/memory than its previous Git-mirroring fleet. These are author-reported system-specific results, not an independent comparison against an offline VCS. Adopt checkout-free APIs and separable derived indexes/caches; do **not** assume its always-online and post-merge-testing choices are general-purpose defaults.

**Meta Sapling/EdenFS.** The [scale overview](https://sapling-scm.com/docs/scale/overview/) traces lazy file/tree/history fetch, watchman/treestate, sparse checkout, virtual materialization and server indexes as separate responses to distinct costs. It also separates commit backup from explicit public integration. Keep lazy materialization optional and offline full clones possible.

**Pijul.** Its [rationale](https://pijul.org/manual/why_pijul) explains change commutation, dependencies and conflict representation. That can preserve identity as independent patches are reordered. It also acknowledges that applying changes correctly does not prove a program behaves correctly. Study patch-centric models without forcing all historical access and binary/rename semantics through patch replay.

**Dolt/Noms.** [Prolly Trees](https://www.dolthub.com/blog/2025-05-16-millions-of-versions/) show history-independent persistent Merkle maps that share pages and support efficient diff. Dolt's [fast merge work](https://www.dolthub.com/blog/2025-07-16-announcing-fast-merge/) shows that page-boundary shifts complicate fast comparisons. Wide-directory formats need real tests, not a reflexive choice of fashionable tree structure.

**SQLite and Git transactions.** [SQLite atomic commit](https://www.sqlite.org/atomiccommit.html) documents durability assumptions. [Git update-ref](https://git-scm.com/docs/git-update-ref) provides conditional reference updates. Neither automatically makes object files, metadata roots, working-directory writes and third-party host events one atomic transaction. Native AVC should own exactly the boundaries it can actually guarantee.

## 4. Where native design can deliver more value

- One authoritative local root transaction joining source operations, workspace generations, recovery pins and retry outcomes, without independently changing Git and jj heads.
- Persistent Merkle directory maps, selective content chunks and index/query layout tuned for local and agent workloads.
- Stable logical change identity, exact immutable revisions, explicit dependency/replacement lineage and first-class conflict objects unrestricted by Git commit headers.
- Checkout-free conditional operations and independent workspaces as fundamental APIs, with no shared mutable directory by default.
- Native quiet backup of private work, efficient object/range sync and target-specific validated publication, without emitting one public push and CI run for each agent attempt.
- Optional scalable server/cache services operating on the same immutable source objects as an offline client.

These are **potential** advantages. Some are already available in parts of existing systems. A new storage format by itself does not make them faster or safer.

## 5. Where native design is worse until proven otherwise

- Larger trusted implementation: canonical encoding, object I/O, GC, concurrent writer protocols, interrupted checkouts, merges, path portability and recovery all require independent validation.
- Git interoperability: original IDs, signing, tags, filters/attributes, LFS, submodules and partial histories may not round-trip without careful conversion.
- Operational maturity: Git and jj benefit from existing user testing, diagnostics, ecosystem support and optimized pack/index behavior.
- More implementation time can still harm reliability if unfinished safety work is rushed or neglected; no deadline does not remove that relationship.
- Merely disjoint textual edits do not imply semantic commutation; a machine attestation is only as strong as what it checked.

**Therefore:** launch via explicit experimental status, separate backups and measured compatibility. Reversible adoption and data export are product features, not afterthoughts.

## 6. Differentiation / adoption gate

Test whether a developer can safely experiment/recover without managing transient commits; whether 8–32 independent workers can explore and compose faster/safer than Git worktrees plus jj; whether agent/CI queries avoid repeated materialization; and whether exact-candidate validation reduces downstream work without compromising correctness. Measure user-facing time, failure/recovery outcomes, CPU/RAM, stored/network bytes, and hosting/CI events.

If results are marginal, evaluate upstream contributions or a smaller complement; do not claim the native engine wins by construction.

## 7. Provisional naming

**Tavi** remains a provisional name and CLI suggestion, not an applied rename. It is short, neutral and pronounceable. An unrelated [TAVI visualization toolkit](https://github.com/neutrons/TAVI) uses the name; the bounded screen did not identify a dominant direct VCS collision. This is not comprehensive name/namespace/legal clearance. Keep AVC until a separate naming decision.
