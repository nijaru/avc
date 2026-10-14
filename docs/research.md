# VCS design investigation: requirements, evidence, and alternatives

**Research date:** 2026-10-14.  
**Status:** Comparative investigation and a scoped reference-design recommendation, not a finding of universal optimality.  
**Related:** [Concrete candidate and contracts](design.md); [qualification and next experiments](roadmap.md); [reproducible contract probes](experiments/vcs_contract_probes.py).

## 1. Conclusions supported by this investigation

A native system remains a legitimate product direction. Current evidence supports independent workspaces, precise machine operations, inexpensive private recovery, immutable source access without checkout, and deliberate integration. It does **not** show that agents require a patch graph, a particular database, a mandatory server, or the previous State/WorkVersion/Application abstraction.

The leading **reference implementation candidate** is a native multiversion repository with persistent source trees, immutable revisions, separately tracked logical-change evolution, resource-scoped transactions, and local or remote access through the same operation contracts. This is an evidence-bounded starting point for the stated general-purpose workload, not a stable format decision or a claim of innovation by itself.

Two complete challengers remain important: a causal change graph with indexed materialization, and a parent-relative changeset log with derived source trees. They can satisfy most of the same user requirements. Their relative merits depend on how often selective change removal/reapplication, deep divergence, source queries, and ephemeral draft writes occur. No measured AVC result currently selects a universal winner.

The previous blanket selection of a change-centric core and the claimed differentiation from naming an Application record are withdrawn. A source revision plus provenance can express the latter without improving the merge algorithm. Retaining the original authored object does not eliminate transformation or validation work on a new base.

## 2. Define the workload before declaring an optimum

These are proposed product workloads, not measured population shares:

| User/workload | Required outcome | Dominant costs or hazards |
| --- | --- | --- |
| Solo developer using ordinary editors and shell tools | Understand what changed; recover experiments; work offline | Surprising state, destructive restore, scanning, unnecessary bookkeeping |
| Team developing features and releases | Review coherent changes; merge and backport predictably | Conflicts, history interpretation, stale review, interoperability |
| Parallel compositional agents | Combine several independently useful results | Workspace interference, stale assumptions, merge/validation cost |
| Parallel alternative-solving agents | Evaluate alternatives and choose one or a subset | Workspace creation, redundant work, retention of failed attempts |
| Sequential agent edit/test loops | Checkpoint frequently without publishing every attempt | Capture latency, write amplification, unbounded recovery, CI amplification |
| CI, search, code review, automation | Read exact files/history or construct a change without a clone | Read fanout, object negotiation, cache misses, checkout overhead |
| Very large monorepos | Bound local data and scale hot shared targets | Data placement, directory/history indexes, integration ordering |
| Projects with assets, submodules, signed history or restricted paths | Preserve bytes and policy; make limitations explicit | Opaque merges, locks, conversion, authentication and access scope |

Competitive alternatives and compositional work must not share an implicit winner-takes-all policy. A workspace optimized for one can discard useful work from the other. Agent scheduling remains outside the VCS; the VCS supplies clear independent work and integration operations.

Hard requirements are source integrity, exact revision interpretation, explicit conflicts, predictable recovery, authorization, and truthful supported-platform behavior. Within those constraints, minimize whole-task latency, resource cost, repeated human/agent work, operational burden and migration friction. There is no defensible single numeric score until workload weights and safety tolerances are specified. A small decrease in conflicts does not compensate for silent wrong merges.

Human needs are not inferred only from fleet engineering reports. The 2026 **Git Takes Two** study emphasizes shared awareness of distributed state in a learning setting [6]. It supports making local, peer and remote state legible, but is not evidence of measured productivity gains for all professional developers. **GitGoodBench** supplies agent tasks and history-manipulation evaluation material [7]; its historical model results are not current agent capability estimates.

## 3. What the recent production reports establish

### Scion: a workload-specific native redesign

Anthropic's October 13 report describes more than 30,000 merges and roughly 25 million CI checkouts per day. Scion combines parent-relative changed-file records, derived indexes, server APIs and tiered immutable-object caches. The authors report roughly 100-fold faster checkouts and a 200-fold smaller CPU/memory footprint than their former mirror infrastructure [1]. These are whole-system, author-reported comparisons. They do not isolate a benefit from patch algebra; Scion's changesets still have parents. Its online-only clients and post-merge/pre-deploy validation are deployment choices, not necessities for ordinary projects.

### GitHub: a crucial counterexample to the necessity claim

GitHub's October 6 report, updated October 7, attributes pressure to rapid push growth, hot refs, read fanout and maintenance. Its redesign separates durable storage from serving capacity, reduces the coordinated part of writes, and moves maintenance off serving workers. It reports **up to 35-fold write-throughput improvement in internal benchmarks**, while retaining Git [2]. That is not a fully rolled-out universal result. It does show that a storage/service redesign and a new change algebra are independent choices.

### GitFarm and Sapling: reuse data rather than repeatedly reconstruct environments

Uber's April 2026 GitFarm preprint describes production use of repository-scoped RPC and warmed execution environments while retaining Git semantics [3]. Its resource savings are explicitly client-side; count the server fleet when comparing total cost. Sapling's scale documentation separates lazy content/history access, incremental working-copy tracking and optional virtual filesystems [4]. These are strong precedents for making materialization optional, not evidence that one canonical source model is optimal.

**Inference:** AVC can pursue better native APIs and storage without needing to prove a new merge calculus first. Conversely, adding a patch graph alone will not solve repeated clones, server fanout, CI jobs, or hot-target scheduling.

## 4. Do not conflate independent design axes

1. **Authoritative history:** source snapshots, parent-relative effects, or a causal edit graph.
2. **Edit correspondence and merge policy:** line/token matching, structural matching, explicit anchors, move provenance, and conflict rules.
3. **Physical representation:** complete objects, compressed deltas, chunks, pages, segments, and databases.
4. **Working environments:** directories, reflinks, overlays, sparse materialization, or virtual filesystems.
5. **Distribution:** complete offline stores, incomplete clients, remote query execution, object caches and replication.
6. **Acceptance:** private drafts, review subjects, validated batches, accepted versus verified targets, and deployment policy.

A snapshot repository can support syntax-aware merge, remote APIs and private recovery. A causal graph can maintain fast materialized snapshots and serve opaque files. A parent-relative changeset log can run locally. None of these properties is exclusive merely because one incumbent combines them a particular way.

## 5. Three concrete competitors

### A. Persistent source revisions with explicit evolution

A revision records a source-tree root, ordered parents, logical change identity and ordinary authorship metadata. An immutable evolution record relates replacements/splits/folds. Workspace and target pointers carry separate generations. Recovery captures have separate retention roots. A merge creates a revision and explicit recipe/provenance; it does not mutate either input revision.

Source reads traverse the tree, not an edit log. Default merge compares base/left/right entries, recurses through directories and applies an explicitly selected text or structural driver. Incompatible types, opaque concurrent replacements and unresolved content become typed conflicts. Private history can be reorganized; accepted targets advance through explicit new revisions or reverts.

**Strongest fit:** ordinary filesystem edits, arbitrary files, exact builds, historical inspection, understandable imported history. **Weakness:** reconstructing two snapshots' relationship does not recover every causal editing intent. Strong selective undo or context-independent replay needs additional history and defined transformation semantics. Changed ancestry may create new revision IDs; stable logical identity alone does not imply commutation.

### B. Causal change graph with materialized views

A change has a content-derived identity, causal dependencies and typed operations. Text references stable content/edge identities; deletion records whose deletion is being expressed, not just a boolean tombstone. Namespace operations need their own concurrent move/copy/type rules. Opaque replacements can use a multivalue conflict representation. Repository history is a causally closed set/graph; an indexed interpreter yields a source tree or explicit conflicts.

Pijul supplies real prior art for change-labelled edges, deleted/retained relationships and conflict graphs [9]. This is not adequately specified by writing `merge = graph union`: the difficult contract is interpreting the union, maintaining indexes, and retaining enough causality for later changes. Unknown filesystem edits also need a diff-to-operation recording policy; the original editor's exact operations cannot generally be inferred from before/after bytes.

**Strongest fit:** repeated integration of long-divergent changes, causal selective undo and resolution reuse, especially when edits are captured through controlled operations. **Weakness:** more causal state to retain/index and more choices when matching external edits to identities. Unbounded offline compatibility and aggressive tombstone/context pruning conflict unless a resynchronization epoch or equivalent protocol is defined. These are tradeoffs, not proof of poor performance or lack of binary support.

### C. Parent-relative changed-file records with derived trees

A changeset records parents and an explicit list of file-level effects, each carrying old/new identifiers and validation rules. A materializer derives tree states and history indexes, eagerly or lazily; periodic durable derived checkpoints bound cold reconstruction. Merge computes a result and records effects against the designated parent. The authoritative effect and any cached result must agree.

**Strongest fit:** high-rate draft creation and centralized indexed services with many histories rarely materialized. **Weakness:** cold historical reads, repair and disconnected work pay materialization costs unless appropriate state is already cached. This is not necessarily online-only, but its economics depend heavily on index hit rates and bounded reconstruction.

### Conditional comparison, not arbitrary scores

| Criterion | A: source revisions | B: causal graph | C: effect log |
| --- | --- | --- | --- |
| Exact files at an arbitrary retained revision | Direct persistent-tree access | Direct when materialized; otherwise graph/index work | Direct when materialized; otherwise parent/checkpoint work |
| Stable identity across reordering | Logical IDs; result revisions may differ | A core benefit under its specified change algebra | Not supplied by the log representation alone |
| Selective undo of independent identical effects | Requires causal provenance and a separate undo contract | Can be native to labelled effect history | Requires extra causal semantics |
| Generic uninstrumented filesystem editing | Natural state capture | Must infer correspondence and emit graph operations | Natural file-level effect capture |
| Cheap updates in very wide directories | Depends on tree pages/commitment | Depends on graph indexes/materialization | Can defer tree materialization |
| Structured/text merge improvements | Independent driver choice | Must compose with graph semantics | Independent driver choice |
| Efficient thin-client access | API/cache design | API/cache design | API/cache design |
| Git interchange | Relatively direct history projection | More causal information lacks a Git counterpart | Parent-relative history projection |
| Retention and compaction | Keep reachable states and required provenance | Also preserve causal references needed by future edits | Keep retained effects and reconstruction checkpoints |

**Current judgment:** A is the strongest reference candidate for the declared broad workload, because it gives an explicit ordinary-file contract without first assuming demand for fine-grained causal undo. B is the strongest semantic challenger. C is the strongest alternative when admitting vast amounts of rarely read draft history dominates. This preference is not an empirical performance ranking. Reverse it when representative traces show another design wins the relevant end-to-end outcomes.

## 6. Papers that materially change the evaluation

### Eg-walker — EuroSys 2025

The peer-reviewed paper and artifact combine an event graph with cached document state and temporary merge machinery. Sequential activity need not keep the full causal machinery resident; concurrent history activates it [10]. This defeats the simplistic assertion that operation history always means replaying everything or retaining per-character working state indefinitely. It remains a text-editing result, not a complete file/namespace VCS or a proof that arbitrary source merges are correct.

### Fugue / FugueMax — published 2025

Work on non-interleaving sequence CRDTs makes ordering quality an explicit property [11]. Deterministic convergence and user-appropriate grouping are distinct. Even a non-interleaving text result can combine logically incompatible code, so neither property substitutes for integration validation.

### Certified Mergeable Replicated Data Types — PLDI 2022

This work provides verification methods for three-way merge of versioned data [12]. Formal merge reasoning is not exclusive to operation-based source control. Its verified data-type laws do not establish correctness of arbitrary programs stored in those data types.

### MergirafSemi — August 2026 preprint

The study evaluates 21,615 scenarios across five languages, comparing semistructured and fully structured merge [13]. Its resolution/missed-conflict findings show why fewer conflicts alone is an inadequate metric. The authors' test- and output-based classifications are approximations; filtering and incomplete tests limit ground truth. It is useful current evidence, not a universal winner across languages or repositories.

### Summer — April 2026 preprint; Merge-Bench — May 2026

Summer explores token-level textual merging without a complete language grammar [14]. Merge-Bench supplies real conflict cases and evaluated model-based resolutions [15]. These broaden the merger comparison beyond diff3 versus a full AST. Their datasets and matching criteria differ; developer-merge agreement is not identical to correctness, and historical model results must not be presented as today's best model performance.

### BranchFS — February 2026 preprint / agentic-OS workshop work

The implementation explores cheap filesystem branching; its proposed OS interface is not an established portable syscall [5]. Its first-successful-commit policy invalidates alternatives, fitting competitive exploration rather than arbitrary parallel feature composition. Use it to evaluate workspace economics, not as a replacement for historical source semantics.

### ForkBase / POS-Tree — VLDB 2018; Dolt fast merge — July 2025

Persistent content-addressed ordered maps can share data across versions [16]. Dolt's engineering discussion also explains complications from shifting page boundaries [17]. These motivate a paged-directory experiment. They do not justify claiming every merge is proportional only to the number of edited entries, or freezing a prolly-tree algorithm before testing ordinary and adversarial layouts.

### GitGoodBench and Git Takes Two

These are sources for task and usability evaluation, not storage rankings [6,7]. A native CLI should be evaluated with equal model/tool budgets and task outcomes rather than punished for having a different command trace. Human evaluation needs first-use and experienced workflows, recovery success and state understanding, not just task-time averages.

## 7. Concrete investigations performed here

The probes are deliberately narrow counterexamples. They are not a native engine, a full merge calculus, performance measurements, or power-loss tests. Environment: Linux, Python 3.13.5, Git 2.47.3. That installed Git version is recorded, not described as current upstream.

| Probe | Observed result | Consequence |
| --- | --- | --- |
| Two branches independently delete the same line; merge; revert one branch's commit | Merge coalesces the identical deletion; inverse content change restores the line | Content revert and causal retraction are different desired contracts, not a Git defect |
| Two branches edit different files; both pass their tests separately | Git merges cleanly; combined test raises TypeError | Mechanical independence is not program compatibility |
| Validate current file; external editor writes; atomically replace path | Intervening edit is overwritten | A pre-overwrite content check cannot provide universal no-clobber without writer coordination |
| Prepare/publish objects versus mark/sweep in a four-event model | Two of six valid schedules leave a dangling root without a staging pin; none do with the model's atomic pin | Pin in-flight objects, not just already published roots; actual persistence still needs testing |
| Two independent workspace updates | One global expected generation spuriously rejects one; scoped generations accept both | Distinguish short physical serialization from logical shared-state conflicts |

The first result is especially relevant to Pijul's deletion discussion [9] and Jujutsu's documented same-change rule [8]. Equal rendered bytes do not mean the same history. A source-tree design may keep provenance too; the result does not prove only a graph can support the desired behavior.

The restore race falsifies an explicit claim in the previous AVC design. A strong guarantee needs exclusive control of the mutable workspace, a suitable platform mechanism with stated assumptions, or non-destructive restoration to a new directory. An API generation token does not stop an editor writing through an already open file descriptor.

Reproduce with `python docs/experiments/vcs_contract_probes.py --output results.json`. Recorded output is [vcs_contract_results.json](experiments/vcs_contract_results.json). These scripts only use fresh temporary directories and never mutate the user's repository.

## 8. Concrete design consequences

### Immutable source access, not mandatory cloning

Use exact revision/tree IDs for `read`, `diff`, `history`, `merge`, `apply` and candidate creation. The same library semantics can execute locally or behind a remote API. Batch object requests and query results; a separate network request per tiny object defeats caching gains. A full or explicitly pinned local working set works offline; an incomplete client reports missing promised data instead of treating it as deletion.

### Recovery, review and acceptance are different

Automatic private captures should not create public refs, reviews, notifications or CI runs. A capture is an observation of admitted files, not unwritten editor buffers or a simultaneous multi-file snapshot unless the capture mechanism establishes that. Named pins differ from expiring captures; sensitive material and build outputs need explicit admission/retention policy.

Do not require fine-grained event history merely to protect local work. Conversely, do not promise arbitrary causal undo after discarding the very history needed to interpret it. Retention is part of semantics, not only storage cleanup.

### Integration capacity and policy

For a serial validated batch of k changes taking time T, a simple sustained capacity bound is approximately k/T, excluding failed batches and other costs. More concurrent producers do not remove that bottleneck. Keep drafting independent, validate immutable batches, reuse checks only with sound input equivalence, and expose queue/backpressure.

A project may instead maintain separate accepted and verified targets or validate after merge and before deployment. Those are explicit policy profiles with different guarantees, not an automatic consequence of agents. GitHub's merge queue [18] is prior art; Bazel's action cache [19] supplies a useful input-based reuse boundary. A VCS alone cannot infer complete build dependencies from changed paths.

### Physical store and namespace commitments

For an executable reference implementation, a single local transactional database containing small objects, roots and request results avoids an initial cross-store durability protocol. It is a correctness/performance baseline, not a commitment that all large assets belong in it. Compare a single append log or staged external segments on the actual limiting workload before stabilizing storage. WAL durability settings, filesystem sync behavior and consistent backups are explicit requirements [20].

Compression, segment placement and physical deltas can change without changing logical IDs. In contrast, changing a canonical tree's paging algorithm or serializing a file as a different chunk manifest may change its digest. Require equal-content/equal-root **within a specified logical format**, or design a separately proven representation-independent commitment. Do not call all tree/chunk changes purely physical.

### Trust, not merely hashes

Separate content integrity, identity authentication, authorization and validation evidence. A local agent with writable access to the repository files can bypass advisory API rules. Repository hooks, parsers and merge plugins must not execute merely because untrusted objects were fetched. Native permission scope must cover the reachable object closure; signatures are not transport confidentiality. The NDSS 2025 gittuf paper is relevant prior art for verifiable repository policy and ref-state history [21].

## 9. What would justify replacing incumbents?

The product hypothesis is lower end-to-end cost and error rates for frequent experimentation, handoff, recovery and integration, with an ordinary single-developer workflow that remains understandable. None of isolated workspaces, stable change IDs, merge queues, content hashes or remote APIs is novel on its own.

The native case becomes compelling if coordinated changes to representation, materialization, recovery and interfaces produce substantial improvements together. It becomes weak if most measured gains come from a small wrapper over an existing engine. Evaluate that honestly without converting migration difficulty or implementation effort into a veto on a better architecture.

Measure workload distributions, not just averages: p50/p95/p99, cold reads, failed attempts, bytes retained after GC, cache-miss traffic, validation compute, review events, offline recovery, bridge cost and operator recovery effort. Preserve each system's semantics in tests; do not count a deterministic but wrong merge as a performance win.

## 10. Decision register

| Concern | Position | Basis / limitation |
| --- | --- | --- |
| Git dependency | Not required; native core remains in scope | Product freedom, not proof of a speed advantage |
| Independent workspaces and scoped writes | Adopt as a contract | Direct isolation and concurrency reasoning; probe |
| No-checkout source API | Adopt as a contract | Scion, GitFarm, Sapling; beneficial across models |
| Separate capture/review/acceptance | Adopt as a contract | Different retention and event needs |
| Snapshot-oriented candidate A | Reference candidate, not stable canonical decision | Strong fit to generic filesystem and exact-source workflows |
| Causal graph B | Serious challenger, not presumed best | Potentially stronger selective-undo/reapplication semantics |
| Changeset-log C | Strong high-rate draft/service challenger | Must measure cold materialization and retained storage |
| State/WorkVersion/Application as a novel core | Withdraw as mandatory | Provenance organization did not prove better transformations |
| Text/structural/LLM merge | Independent evaluated components | Minimize wrong merges, not conflict count alone |
| SQLite/segments/log | Reference backend plus focused measured challengers | No comparative native durability or speed result yet |
| Tree pages / chunks / hashing | Format choices need explicit vectors and tests | Equal-content identity must be qualified by format |
| Universal no-clobber inplace restore | Reject under uncoordinated writers | Reproduced race; explicit ownership/recovery mode required |
| Universal optimality / native performance claims | Not established | Requires implemented competitors and workload evidence |

## Primary sources

Dates refer to the specific publication/version reviewed, not an assertion that every component is the newest release. Living documentation was consulted on 2026-10-14. Production results are author-reported; preprints are distinguished from peer-reviewed papers. No new user interviews or representative population survey were performed.

1. Anthropic, **Redesigning source control at Anthropic**, 2026-10-13: https://www.anthropic.com/engineering/redesigning-source-control
2. GitHub, **Building Git infrastructure for agent-scale development**, 2026-10-06, updated 10-07: https://github.blog/engineering/architecture-optimization/building-git-infrastructure-for-agent-scale-development/
3. Dwivedi et al., **GitFarm**, preprint v1, 2026-04-13: https://arxiv.org/html/2604.11977v1
4. Sapling, **Scaling source control**, living architecture: https://sapling-scm.com/docs/scale/overview/
5. **Fork, Explore, Commit: OS Primitives for Agentic Exploration**, BranchFS, preprint v1, 2026-02: https://arxiv.org/html/2602.08199v1
6. **Git Takes Two**, preprint v1, 2026-02: https://arxiv.org/html/2602.19714v1
7. **GitGoodBench**, 2025 paper: https://arxiv.org/html/2505.22583v1
8. Jujutsu, **Technical conflict model**, including same-change rule: https://docs.jj-vcs.dev/latest/technical/conflicts/ ; architecture: https://docs.jj-vcs.dev/latest/technical/architecture/
9. Pijul, **Theory**, living manual: https://pijul.org/manual/theory.html ; Darcs conflict representation: https://darcs.net/Theory/Conflictors
10. Gentle and Kleppmann, **Collaborative Text Editing with Eg-walker**, EuroSys 2025; paper/artifact index: https://martin.kleppmann.com/2025/03/30/eg-walker-collaborative-text.html ; examined manuscript: https://arxiv.org/html/2409.14252v1
11. **The Art of the Fugue: Minimizing Interleaving in Collaborative Text Editing**, manuscript: https://arxiv.org/abs/2305.00583
12. **Certified Mergeable Replicated Data Types**, PLDI 2022 extended version: https://arxiv.org/html/2203.14518v1
13. **MergirafSemi: A Language-Agnostic Semistructured Merge Tool**, preprint, 2026-08: https://arxiv.org/abs/2608.11345 ; Mergiraf implementation architecture: https://mergiraf.org/architecture.html
14. **Summer: A Universal Textual Merge Strategy Based on Tokens**, preprint v1, 2026-04: https://arxiv.org/html/2604.13813v1
15. **Merge-Bench**, 2026-05 manuscript: https://arxiv.org/html/2605.25890v1
16. Wang et al., **ForkBase**, PVLDB 2018: https://www.vldb.org/pvldb/vol11/p1137-wang.pdf
17. Dolt, **Announcing Fast Merge**, 2025-07-16: https://www.dolthub.com/blog/2025-07-16-announcing-fast-merge/
18. GitHub, **Managing a merge queue**: https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/configuring-pull-request-merges/managing-a-merge-queue
19. Bazel, **Remote caching**: https://bazel.build/remote/caching
20. SQLite, **WAL**: https://www.sqlite.org/wal.html ; **Atomic commit**: https://www.sqlite.org/atomiccommit.html ; **synchronous**: https://www.sqlite.org/pragma.html#pragma_synchronous
21. **Rethinking Trust in Forge-Based Git Security**, NDSS 2025: https://www.ndss-symposium.org/wp-content/uploads/2025-1008-paper.pdf
22. Mohan et al., **Finding and Understanding Bugs in File Systems**, CrashMonkey/ACE, OSDI 2018: https://www.usenix.org/conference/osdi18/presentation/mohan
23. Git, **Pack format**: https://git-scm.com/docs/gitformat-pack ; **Conditional ref updates**: https://git-scm.com/docs/git-update-ref

## Scope of the completed work

Completed: primary-source comparison, inspection of the current repository design, three concrete architecture candidates, a scoped reference design, and five reproducible contract probes. Not completed: implementation or benchmark of a full native engine, Pijul/Jujutsu runtime comparison, power-loss verification, or a representative user trial. The contract probes eliminate several unsound assumptions but cannot rank complete VCS implementations.
