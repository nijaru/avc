# AVC research and architecture decision

**Updated 2026-10-14.** These are primary-source observations, engineering deductions and open hypotheses—not native AVC performance measurements. The proposed behavior lives in [design.md](design.md); implementation/verification gates live in [roadmap.md](roadmap.md).

## 1. Conclusion: a direction, not a solved optimum

There is now a **coherent implementable native VCS direction**: small immutable parent-relative changesets and blobs as source truth, with versioned persistent Merkle source-tree views and additional query indexes computed when useful; full offline development and optional scalable server execution use the same semantics. Independent workspaces mutate independent roots; only genuinely shared targets require agreement.

**Why lead with this:** Scion and Mononoke independently demonstrate the design at substantial production scale. Cheap change admission and separately evolvable derived structures match increasingly frequent agent edits and no-checkout CI queries. This is evidence of feasibility and value for those workloads, **not proof that the format outperforms snapshot-root commits for ordinary repositories**.

The strongest alternative remains a canonical persistent snapshot tree per revision with changed-file hints. A causal patch graph offers different semantics for selective undo and commutation, but its benefits cannot be asserted merely because agents run concurrently. The right choice is the one that wins full user work, including cold reads, rebuilds, merges, retention, transfer and recovery.

## 2. Ground truth: workloads and users

The architecture must handle three genuinely different multiworker situations:

| Situation | Actual requirement | Wrong optimization |
| --- | --- | --- |
| Sequential edit/test/fix | Very frequent private recovery, cheap status, reliable undo | Public commit/push/CI for every intermediate edit |
| Competitive attempts | Cheap isolated alternatives; choose and retain desired results | Implicitly merge every failed alternative |
| Parallel useful features | Workers progress independently; explicit combined validation | One global workspace/branch lock, or first-winner semantics |

A solo developer still needs understandable history, ordinary editors, offline operation and reliable data recovery. Agent infrastructure adds read/query fanout and concurrent speculative writes, but the source-control engine should not own agent prompts, task scheduling, process sandboxes or every build artifact.

**Published deployment observations:** Anthropic reported about 30,000 merges/day and 25 million CI checkouts/day for its atypically large monorepo, plus strong infrastructure savings *against its previous mirror fleet*. [Scion article, October 13, 2026](https://www.anthropic.com/engineering/redesigning-source-control). GitHub reported major growth in commits, pushes, merges, CI fanout and service costs, while investing in scalable storage **without changing Git semantics**; its up-to-35× throughput figure is an internal benchmark, not a universal result. [GitHub engineering, October 6–7, 2026](https://github.blog/engineering/architecture-optimization/building-git-infrastructure-for-agent-scale-development/).

**Inference:** changed-file storage is one way to remove hot-path computation. Other important costs—network fanout, redundant checkouts, CI, maintenance and target coordination—can be addressed independently of the canonical history model. Do not count all agent-generated commits as inherently expensive or infer that a native VCS automatically reduces host load.

## 3. What Scion and Mononoke actually choose

### Scion (Anthropic)

[Primary source, October 13, 2026](https://www.anthropic.com/engineering/redesigning-source-control):

- Canonical changeset: commit metadata, parents, and list of files changed **relative to the first parent**. File contents are content-addressed.
- The changeset is a write-ahead-log entry; creating it does not require updating a ref or building Git packfiles. A public ref update is a separate CAS operation.
- Trees, ancestry acceleration, blame, file history and path/copy indexes are recursively **derived** from changesets. Computation may be eager or lazy.
- API/server performs commits, merges, rebases, history/diff queries. Thin clients cache selected objects and materialize only requested workspaces.
- Immutable requests flow through node-local daemons, regional caches and authoritative services, avoiding the full repository/pack negotiation for each job; caches have topology/failure-isolation rules.
- Scion optimizes an always-online, centralized fleet. Its measured 100× fleet checkout and 200× infrastructure footprint comparisons are not source-format microbenchmarks, and do not establish the best local offline format.

**Design consequences:** use parent-relative changes with a separately cached source view; put query and materialization APIs below the CLI; keep immutable object fetch distinct from mutable target CAS. Do **not** require a service for ordinary work. Do not adopt Scion's post-merge/pre-deploy policy as every project's default.

### Bonsai / Mononoke (Meta)

[Mononoke architecture overview](https://github.com/facebook/sapling/blob/main/eden/mononoke/docs/1.3-architecture-overview.md), [Bonsai data model](https://github.com/facebook/sapling/blob/main/eden/mononoke/docs/2.1-bonsai-data-model.md), [derived data](https://github.com/facebook/sapling/blob/main/eden/mononoke/docs/2.3-derived-data.md), consulted 2026-10-14:

- Bonsai is VCS-independent canonical history: parent IDs, metadata, and a flat mapping of changed full file paths; content is stored separately. The first-parent notion is central to reconstructing resulting files.
- It preserves file-kind and copy information, supports snapshot-related changes and **subtree changes** for certain copies/merges. That last detail is consequential: naïve flat file deltas otherwise make a large directory move touch many paths.
- Manifests, ancestry/path indices, blame, and Git/Hg representations are versioned derived data computed outside the minimal ingest path.
- Immutable blobstore and mutable bookmark/metadata DB have separate ownership; the server can scale file services, landing and derivation independently.

**Design consequences:** account for path/tree-change operations before freezing our changeset format. The initial prototype can enumerate affected paths, but must benchmark large renames and decide whether an exact subtree operation is warranted. Copy metadata is not automatically proof of author intent.

### The distinguishing unanswered issue

Both systems demonstrate **cheap admission of increments plus derived state** at scale. Neither proves that deferring a source-tree root is better when a solo user immediately diffs, builds, tests, and checks out almost every revision. That is the direct test against snapshot-first storage.

## 4. Competing complete canonical representations

| Model | Upside | Downside / hidden work | Decision posture |
| --- | --- | --- | --- |
| **C — Parent-relative changeset truth + derived trees** | Small canonical write, cheap skipped drafts, history/index format independent of changeset identity, incremental remote service | Cold revision reconstruction; derived index lag; directory moves may require many path entries; cache/checkpoint policy is essential | **Selected reference, not frozen** |
| **A — Canonical tree-root revisions + changed-path index** | Complete source view immediately, Merkle subtree diff, easy opaque files and bulk directory restructuring, predictable offline historical reads | Tree construction in every revision write; changing canonical tree encoding affects object identity | Mandatory challenger |
| **B — Causal patch graph + materialized working view** | Strong selective undo, dependencies and commuting changes in supported operations | Context/identity semantics for arbitrary filesystem edits, copy/move, binaries, deletion, conflict reuse, GC and arbitrary cold reads | Challenger only if causal semantics earn their additional cost |

This is not a preference for novelty. Git already provides structural sharing, object hashes and highly optimized pack/index maintenance. Jujutsu already provides stable change identity, operation history, automatic working-copy snapshots and conflict-bearing commits. **AVC does not become differentiated simply by reproducing those features in Rust.**

For model C, a merge with multiple parents stores a **complete first-parent delta for its resulting tree**; other parents express ancestry and provenance, not an instruction to replay all their edits. This yields deterministic reconstruction while allowing independent merge-policy improvement. A stable ChangeId through amendments/rebase is metadata, not evidence of algebraic commutation.

For model A, the reference comparison must use the *same* file/blob objects, status workflow, merger, query API and SQLite durability level. Only the canonical revision record differs. Otherwise the experiment cannot attribute a benefit to the representation.

For model B, a true alternative must define anchor/change identity, dependency closure, inversion, concurrency and file/path operations; a generic text diff with IDs is not a patch calculus. [Pijul manual](https://pijul.org/manual/theory), [Darcs theory](https://darcs.net/Theory/MergersDocumentation). Pijul's site states strong commutation/conflict guarantees under its chosen model; these are model-specific, not proof of general software semantics or AVC improvement.

### Cost model and edge cases

Let k be changed paths, d effective directory depth, and H the unmaterialized sequence of changes between a requested revision and the nearest cached source view.

- C canonical admission writes approximately k changed-path records plus changed file bytes, excluding filesystem change discovery and durability cost. Derived tree construction later touches changed paths and ancestors.
- A admission writes changed file bytes plus the needed persistent tree updates, roughly proportional to affected nodes, not a full copy of N source files.
- C cold reconstruction may require work proportional to all relevant changes through H; efficient source checkpoints and incremental derived trees are therefore important. If every revision is immediately queried, C may move rather than remove work.
- A and C both need real merging; C's merge file delta may be larger than the set of conflicts, since it describes the entire result relative to the chosen first parent.
- C's naïve directory move is O(number of affected files); A can often reuse a whole subtree reference. Test explicit subtree-change semantics without compromising generic path behavior.

Neither format inherently solves checkout-frees APIs, history index quality, cache topology, or shared target throughput. Source-state derivation and merge semantics must be deterministic and independently testable.

## 5. DSA choices: choose per question, not by popularity

| Problem | Leading structures | Caution |
| --- | --- | --- |
| Source namespace | Persistent hierarchical Merkle directory tree, small sorted flat nodes | Wide directories and moves need dedicated tests |
| Huge ordered maps | POS/prolly tree, canonical Merkle B+ tree, compressed radix trie | Boundary shifts, object count, cold locality and history-independent roots |
| Ancestry | Immutable parent DAG plus generation numbers, skip links, selective reachability bitmap | Index is derived and can be rebuilt; merge bases still need complete correctness |
| Path history | Exact path postings and changed-path Bloom filters | Bloom is a negative filter, not source truth; renames need provenance |
| Blame | Derived line-origin ranges with versioned algorithms | Incorrect attribution is possible with moves/copies and differing algorithms |
| Files | Whole content blobs; selective chunking or physical deltas | Compression/chunking not intrinsically good for tiny or already-compressed files |
| Object placement | Small transactional store, later immutable segments/append-only journal | Separate CAS from physical durability; GC of staged objects |
| Synchronization | Batched immutable object/tree requests, resumable bundles and caches | Avoid per-object network RTT and unbounded lazy reads |
| Concurrency | Per-workspace generations and per-target CAS, immutable operations | Physically serialized SQLite writes differ from logically shared-head contention |

[Git commit-graph format](https://git-scm.com/docs/commit-graph-format) describes generations, first-parent changed-path Bloom filters and commit-indexed parent offsets. [Dolt's fast-merge explanation, July 2025](https://www.dolthub.com/blog/2025-07-16-announcing-fast-merge/) shows why prolly page-boundary shifts require key-range-aware merge algorithms; a Merkle root alone does not make all merge costs trivial. [ForkBase/POS-Tree paper, 2018](https://www.vldb.org/pvldb/vol11/p1137-wang.pdf) provides broader versioned-index evidence.

**No hash/tree/physical store algorithm is yet selected as globally optimal.** Choose a simple canonical codec and reference tree for the first correctness slice, then benchmark the small set of plausible alternatives. File-content hashes and changeset identities are distinct from transport and Git SHA-1/SHA-256 identities.

## 6. Correctness, merge and agent-specific boundaries

**Three-way merge is the conservative default**, including deterministic tree-level type/delete conflicts and first-class conflict objects. It does not guarantee semantic correctness. Token- and syntax-aware drivers may reduce nuisance conflicts but can create incorrect automatic merges. Evaluate both error classes, not just the no-conflict rate. Research references: [Mergiraf architecture](https://mergiraf.org/architecture.html), [Eg-walker, EuroSys 2025](https://martin.kleppmann.com/2025/03/30/eg-walker-collaborative-text.html), [MergirafSemi 2026](https://arxiv.org/abs/2608.11345). Eg-walker studies collaborative text event-history algorithms; its benefits do not automatically transfer to arbitrary file-level VCS operations.

**Independent workspaces** should share immutable data but not mutable directories. Agent identity is not a workspace/session identity. [Trellis ADR 0038](https://github.com/trentbrew/trellis/blob/main/docs/adr/0038-git-authoritative-file-tier.md) reports a real project failure class from competing filesystem/op-log authority. That is a warning about *any* stale restore path, not proof that Git is always the correct native store.

**Recovery** saves only deliberately admitted files, labels capture completeness, protects against lost responses and bounds retention. A portable check-then-replace is not a no-clobber guarantee under uncontrolled concurrent editors. Restore into a separate directory or require actual exclusion of writers. No format removes this filesystem race.

**Integration** verifies exact candidate source against current target policy. Two agents can modify disjoint files and break a cross-file API; clean merge is not adequate proof. CI, code search, review, artifact caching and notifications are distinct event/compute budgets. A target update has an irreducible order; most other object creation and query work does not.

**Publication authority** is separate from private capture/draft sync. Access to content hashes does not confer read/write authorization; directory- and tenant-scope constraints require server enforcement. Cross-host Git adapters cannot claim one transaction across independent authorities.

## 7. What has actually been verified in AVC

The current Rust crate is **still a Git command wrapper with a separately appended JSON operation log**. It is not the native system described above. No native throughput, durability, or interoperability benchmark exists.

The maintained [Python contract probes](experiments/vcs_contract_probes.py) and [recorded results](experiments/vcs_contract_results.json) demonstrate five narrow counterexamples: identical deletion/revert semantics, cross-file semantic interference, a filesystem check/replace race, staged-object GC race in a toy model, and unnecessary global-generation conflicts. These are useful test specifications, not source-model performance evidence.

The paper/engineering posts establish **what others chose and observed**, not an AVC benchmark. Some claims are author-reported fleet measurements, some are formal properties of a restricted model, and some are design inferences. Keep those categories separate.

## 8. Decision register and overturn conditions

| Decision | Status | What would overturn it |
| --- | --- | --- |
| Native VCS rather than Git/JJ wrapper | Product direction | User tasks prove insufficient differentiation or impossibly poor interop |
| Canonical parent-relative changesets + derived trees | **Leading reference** | A beats it on representative full lifecycle or B delivers decisive causal workflow benefits |
| Offline local operation, online thin-client option | Required | Product scope fundamentally changes |
| No checkout required for queries/writes | Required API capability | A specific operation needs a filesystem, documented explicitly |
| Resource-scoped roots/target CAS | Required concurrency behavior | Correctness counterexample exposes insufficient scope |
| Private bounded recovery | Required UX, retention and privacy | User evidence changes admission/retention policy |
| Conservative typed conflicts | Required safety | A formally justified and empirically safer merge alternative |
| SQLite local small-object reference | Implementation starting point | Fault/throughput tests favor journal/segments |
| Deterministic derived tree encoding | Open format | Benchmarks favor another structure; derive version changes safely |
| Subtree moves/large binaries | Open canonical semantics | Large-directory/multi-language project fixtures reveal requirements |
| Native synchronization and Git bridge | Planned capabilities | Adoption/ops research calls for changed priority |
| Project name | Unresolved | User selection after name collision review |

**Next engineering question:** build the smallest credible C-versus-A reference with identical source semantics and durable operation outcomes. Verify cold historical reads, first materialization, repeated tiny edits, GC and bulk directory changes. Only then should AVC stabilize a changeset format.

No additional accumulating decision documents are needed; the maintained design, this research, roadmap and narrowly scoped executable probes are the owners of their respective concerns.
