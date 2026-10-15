# AVC research: choosing a native history model

**Reassessed:** October 14, 2026  
**Result:** source-root snapshots are the strongest **reference** for AVC's broad local and remote workloads. This is an architectural inference and implementation target, **not** an AVC benchmark. [Design](design.md) owns behavior and invariants; [roadmap](roadmap.md) owns experiments and delivery.

## 1. Reopen the previous decision rather than inherit it

The earlier AVC research treated Scion and Mononoke's canonical changed-file formats as evidence that **AVC should also be changeset-first**. That argument skipped differences in workloads **and double-counted related architectural ideas as though they had been independently discovered**.

**Evidence and attribution note (October 14, 2026):** [Rain's first-person account](https://sunshowers.io/posts/anthropic-mononoke/), from an engineer who helped build Mononoke, argues that Scion was heavily inspired by Mononoke and that the acknowledgment was insufficiently specific. Anthropic's [Scion article](https://www.anthropic.com/engineering/redesigning-source-control) does explicitly cite Sapling, which includes the Mononoke server. Mononoke's [Bonsai changeset implementation](https://github.com/facebook/sapling/blob/main/eden/mononoke/mononoke_types/src/bonsai_changeset.rs) and [derived-data documentation](https://github.com/facebook/sapling/blob/main/eden/mononoke/docs/2.3-derived-data.md) establish concrete technical similarities. **They do not establish how Anthropic arrived at its design, whether code was copied, or how much was independently developed.** Rain's interpretation and the varied [Hacker News discussion](https://news.ycombinator.com/item?id=50094694) are useful context, not adjudicated provenance or performance evidence. Treat the deployments as separate operational evidence for related techniques, without counting similarity itself as proof that changeset-first history is optimal.

Anthropic explicitly says a self-contained Git-like offline model is right for most repositories, while Scion's centralized enormous monorepo is unusual. Its reported costs include tens of millions of checkouts/day, 100-GB-scale history and highly online agents. Its first-parent encoding is described as stylistic and adapted to trunk-oriented history. Neither Scion's metrics nor Meta's success isolate a speed advantage of storing deltas *instead of* canonical source roots. [Scion, Oct 13 2026](https://www.anthropic.com/engineering/redesigning-source-control).

Our target is broader: ordinary local editing, non-networked developers, VPSs, remotely executing agents, short-lived containers/microVMs and large hosted services. Most **meaningful accepted revisions** will be inspected or tested. Most **frequent intermediate edits** should instead live in private, bounded recovery state.

This yields a different design hypothesis: keep authored revisions attached to exact canonical source trees; make *temporary attempts, change summaries, sync transfers and query indexes* incremental as needed. Do not conflate capture frequency with permanent history creation.

The previous experiment-free flip toward causal patch graphs was also unsupported. They offer distinct algebraic semantics, not a necessary response to growing agent concurrency.

## 2. Score decisions by complete workflows

The reference must make the following tasks correct and efficient:

| Case | Relevant outcome | Leading model property |
| --- | --- | --- |
| Solo, normal editor, offline | Recover current files; inspect arbitrary old revision immediately | Source-root commitment directly names exact files |
| Iterative agent editing/testing | Retain selected work without 10,000 permanent commits | Separate bounded recovery/capture lifecycle |
| Competing agents | Cheap isolated tries; select the winning solution | Independent workspaces + immutable shared source |
| Multiple useful agents | Compose chosen work, detect conflicts, validate exact combined source | Merge semantics + tested StateId, not representation alone |
| Container/microVM/CI | Avoid full history transfer; fetch exactly required files | Merkle subtree lookups, sparse native RPC + batch transfer |
| Remote server/monorepo | Admit changes without global coordination; cache common reads | Immutable objects and per-target authority, independent of commit encoding |
| Very large directories/renames | Avoid per-descendant rewrite and copy | Persistent subtree reference with bounded directory nodes |
| Crash/data loss | Acknowledged revisions reconstruct and restore independently | Closure durability, recovery, backups; exact source roots help verification |

**Counterfactual:** If AVC's principal users instead create vast numbers of *durable immutable authored revisions* that are never read or tested, Scion/Mononoke-like canonical deltas may win substantially. Prove that workload rather than assuming it describes every agent edit.

## 3. Compare canonical models, with their actual work

### A. Snapshot-root revision DAG — leading reference

Each immutable revision contains an exact content-addressed **StateId** and ancestry. Unchanged blobs/subtrees are shared. The StateId is directly usable for lookup, diff, tests and validation. Changed-path summaries and intent annotations can be derived or verified against states.

**Strengths:** no replay of unbounded predecessors; cheap subtree structural equality and reuse; simple trust/recovery boundary; partial fetch by root; works for editors that expose only final bytes. Merge and cherry-pick outputs are newly constructed exact states. Source identity is separate from logical change identity.

**Costs:** constructing a source root eagerly touches affected persistent directory nodes; canonical tree/hash encoding becomes a compatibility commitment; a huge unpaged directory can have pathological write amplification. Capture/edit scans still require correct filesystem race semantics. Structural snapshots do not encode causal selective undo.

**A isn't physically copying the whole tree per revision.** The cost is writing new blobs and nodes *along changed paths*. In a suitable bounded persistent map, a changed key updates approximately the pages on its search path; bulk multi-path updates share affected ancestors.

### B. Parent-relative changed-file history — serious challenger

Scion and Mononoke/Bonsai make changed-file lists authoritative and derive source views. For k changed paths, accepting the record needn't synchronously rewrite source-tree pages. An index can be versioned and backfilled without rewriting historical changeset IDs.

**Strengths:** reduced immutable revision-admission work for rarely consumed drafts; server write path decoupled from expensive path/blame/source-tree indexes; freedom to replace derived tree encodings.

**Costs:** historical source reads must find/build a view. If every accepted revision is immediately tested, index construction still occurs, now off-path or before tests. Index evictions introduce cold tail latency and rebuild/availability obligations. A flat changed-path encoding must handle large directory operations somehow; its first-parent merge records must represent the exact resulting source. Validating a new change against an old parent's namespace may still require reading the parent's derived state.

**Key insight:** deriveable blame and path history are **also possible under A**. Making changesets canonical is not necessary to have versioned derived indexes, server-side queries, sparse working copies or cheap immutable object admission.

**Scion footnote:** first-parent-relative files are a trunk-oriented choice, not a claimed algebraic breakthrough. Its server executes merges, read queries and writes on behalf of online clients. Those policies are independent of which history record is canonical.

### C. Causal change graph / patch algebra — genuine alternative

Pijul and Darcs model change dependencies and commutation; Pijul preserves conflict-bearing graph states. [Pijul theory](https://pijul.org/manual/theory.html), [Darcs patch theory](https://darcs.net/Theory/MergersDocumentation).

**Strengths:** formally modeled composition for supported operations, selective retraction of particular changes, resolution provenance and independence of some reapplication orders.

**Costs:** the operation representation and anchor identities must be well-defined for normal editors, binary files, moving/deleting trees and partially observed changes. A file diff does not uniquely encode the edit process that produced it. Old graph references, garbage collection, replay and translation from arbitrary bytes all need explicit semantics. A commutative source graph cannot guarantee two program edits will still compile together.

**Overturn condition:** realistic long-lived offline/agent histories show materially better *correct merge and causal selective undo* behavior than A, enough to justify the additional source authority. A trivial example of disjoint text insertions is insufficient.

### Decision table

| Requirement | A: snapshot | B: changeset | C: causal |
| --- | --- | --- | --- |
| Direct complete historical source | Native | Indexed/reconstructed | Materialized view required |
| Cheap write if source never queried | Tree update required | Natural advantage | Often advantageous but graph update still required |
| Directory move reusing subtree | Native binding update | Needs subtree op or many path entries | Requires explicit namespace algebra |
| Identical source dedup across histories | StateId root | Available through derived view | Available if materialized/canonicalized |
| Fine-grained causal undo | Not guaranteed | Not guaranteed | Core strength for supported edits |
| Arbitrary editors and binaries | Direct result | Direct result | Intent ambiguity |
| Index format evolution | Canonical tree migration matters | Easier for entirely derived trees | Graph schema migration matters |
| Hosting without local clone | Possible | Possible | Possible |
| Coordination for independent workspaces | Not inherent | Not inherent | Not inherent |
| Global target validation | Required separately | Required separately | Required separately |

There is no metric-free universal ranking. **Reference A is chosen because direct source identity is a high-value invariant for the broadest tasks, while cheap speculative edits can be handled separately from durable authored history.**

## 4. What established systems actually validate

### Git and GitHub

Git uses immutable Merkle trees and content-addressed blobs; packs/deltas are physical compression, not repeated physical full snapshots. [Git objects](https://git-scm.com/book/en/v2/Git-Internals-Git-Objects); [pack format](https://git-scm.com/docs/gitformat-pack).

Git's modern [merge-tree --write-tree](https://git-scm.com/docs/git-merge-tree) performs a normal capable tree merge **without checking out files or modifying the index**. It supports recursive merge bases, renames and directory/file conflicts. Thus checkout-free merging, content addressing and metadata-only operation are **not novel** by themselves. Git [commit-graph indexes](https://git-scm.com/docs/git-commit-graph) use generations and changed-path Bloom filters to speed history queries.

GitHub's [Oct 6 2026 infrastructure writeup](https://github.blog/engineering/architecture-optimization/building-git-infrastructure-for-agent-scale-development/) describes scaling writes/queries while preserving Git. Evidence against attributing all server throughput limits to snapshot commits.

**AVC is intentionally not Git-based.** A native tree/revision schema can avoid Git's representational conventions and support typed conflict/empty-directory semantics, but must demonstrate real user benefit versus Git plus Jujutsu rather than treating a novel format as a feature.

### Jujutsu

[Jujutsu architecture](https://docs.jj-vcs.dev/latest/technical/architecture/), [first-class conflicts](https://docs.jj-vcs.dev/latest/conflicts/) and [operation log](https://docs.jj-vcs.dev/latest/operation-log/) already establish stable logical ChangeIds, immutable rewritten revisions, undo, concurrent views and conflict-bearing source. Jujutsu is the strongest user-facing baseline, even if AVC uses its own engine.

**Different choice:** our recovery captures need not always be semantic authored revisions; per-workspace resource authority can avoid globally mutable operation-head contention; local/remote source APIs and thin agent clients should be core. These are hypotheses about a better *complete workflow*, not established novel algorithms.

### Scion

[Anthropic Oct 13 2026](https://www.anthropic.com/engineering/redesigning-source-control) emphasizes always-online asymmetric clients, first-parent deltas, derived indexes, narrow ref CAS, tree/blob object request protocol and >99%-hit cache tiers. It reports around 25M CI checkouts/day and 30k accepted changes/day, with major total infrastructure savings against its previous mirrors. Those numbers **do not compare B versus A under identical execution/cache/durability**. Its own conclusion: not a general-purpose replacement for Git.

The relevant lessons are to separate immutable queries from working copies, skip unnecessary materialization, share caches, and keep shared reference coordination narrow. All can be designed above A. Scion and Mononoke provide **different production experience with technically similar models**; their design ancestry is not established by public material, and neither is a controlled comparison against canonical snapshots. Anthropic explicitly characterizes first-parent-relative change recording as a *stylistic choice* for its trunk-oriented use, not a universal theorem about the best canonical history model.

### Mononoke/Bonsai (verified implementation)

[Bonsai data model](https://github.com/facebook/sapling/blob/main/eden/mononoke/docs/2.1-bonsai-data-model.md), [derived data](https://github.com/facebook/sapling/blob/main/eden/mononoke/docs/2.3-derived-data.md), [SubtreeChange Rust code](https://github.com/facebook/sapling/blob/main/eden/mononoke/mononoke_types/src/subtree_change.rs). Canonical changed-path history and asynchronously maintained manifests, path indexes and Git/Hg representations scale Meta's service. Note especially the *distinction* between subtree copy operations that alter manifested source and history-only subtree operations; do not infer every subtree metadata type implements efficient arbitrary directory moves.

Its Sharded Map V2 uses path-compressed trie nodes, inline small terminals and bounded node sizes. This is good evidence for the **source-directory index shape**, regardless of A or B.

### Cursor Continuity: an independently different scaling design

[Git at any scale (Vicent Martí, August 18, 2026)](https://cursor.com/blog/git-at-any-scale) describes **Git-logical source history with S3-backed write-ahead-log authority**, CAS publication, and local Git replicas treated as caches. It batches writes, selectively reconstructs repositories, and scales read-serving replicas without making a new history representation canonical. Published performance figures are author-reported and tied to Cursor's workload and storage class; they do not benchmark native AVC.

**Design consequence:** the failure/serving/transaction boundary is independent of whether a revision commits a tree or a changed-file list. This is a counterexample to claiming that agent-scale hosting logically entails canonical deltas. It also illustrates the tradeoff between reusing mature source operations and accepting Git format constraints. AVC remains independent of those constraints.

### Enroute

[Storage architecture](https://github.com/enroute-sh/enroute/blob/main/docs/internals/storage.md) and [read path](https://github.com/enroute-sh/enroute/blob/main/docs/internals/read-path.md) demonstrate indexed immutable layers, precomputed fetch-object segments and narrow coordinated metadata updates with Git semantics. Some dense IDs are globally allocated through Raft to survive compaction. Their benchmarks largely concern high-rate independent pushes.

**Take only the abstract access-pattern lesson:** sparse object lookup and cold bulk transfer benefit from different physical layouts; logical immutable object IDs must not require coordination. AVC should not select commit segments, Raft, particular layer formats or global dense numbering because Enroute implemented them.

### Pijul/Darcs

These are evidence of real causal change semantics, **not** an implicit requirement for agent concurrency. [Pijul](https://pijul.org/manual/theory.html) also materializes a pristine database; "patch based" doesn't mean absent snapshots or absent indexes. See model C above.

### Recent adjacent research

[BranchFS, Feb 2026](https://arxiv.org/abs/2602.08199) demonstrates fast process/filesystem exploration branches; first-commit-wins is appropriate to *competitive* attempts, not mandatory composition of multiple useful agent tasks. Store process/VM/environment state outside source history.

[MergirafSemi, Aug 2026](https://arxiv.org/abs/2608.11345) reports a trade-off between false textual conflicts and incorrect aggressive auto-merges. The error rate of silently wrong resolutions matters more than maximizing clean merges. [Jujutsu conflict model](https://docs.jj-vcs.dev/latest/technical/conflicts/) supports multiway symbolic source conflicts.

[Persistent ordered-map merge, Dolt 2025](https://www.dolthub.com/blog/2025-07-16-announcing-fast-merge/) shows both the benefits and boundary-shift pitfalls of prolly trees. Choose bounded directory pages based on actual path and cold-query workloads, not a fashionable structure.

[SQLite WAL/synchronous docs](https://www.sqlite.org/pragma.html#pragma_synchronous) describe local full transaction durability assumptions. One local DB is a strong *reference implementation*, not a measured winner over append-only logs.

### Do not conflate architectural similarity with a winning comparison

| System | Canonical source/history | Main observed deployment/optimization | What it establishes |
| --- | --- | --- | --- |
| Mononoke/Bonsai | Immutable changed-file history + derived manifests | Large Meta source-control backend | A production-proven minimal canonical core and versioned derived data |
| Scion | Technically similar changeset/derived-data model | Always-online clients, sparse source APIs, tiered caches, high-rate agent fleet | A distinct operational setting; the degree of direct influence is disputed and cannot be established from public docs alone |
| Cursor Continuity | Git source graph; object-store WAL authority | Consistent Git hosting and dynamically served warm replicas | A different serving design can scale without changing canonical history |
| Enroute | Git source graph; indexed immutable physical layers | High-rate independent pushes and ordered streaming | Physical layout and coordination can change independently of history semantics |

These systems make the case for immutable data, narrow coordinated roots, and query/transfer-aware indexing. They **do not determine AVC's canonical history, directory-node layout, or storage engine**. Compare *complete tasks* with the same correctness and durability guarantees.

## 5. Consequences for DSA and execution layout

**Canonical logical model:** immutable typed Merkle source tree + revision DAG + stable logical change identity + resource-scoped operation authority. This makes the common answer "what source exactly?" direct.

**Tree DSA:** start from a simple sorted directory baseline to check semantics. Before stable format, compare deterministic radix/sharded nodes (bounded pages, small terminals) with ordered POS/prolly pages on real filesystems, huge directory renames, path lookup, sparse diff, cold remote requests and adversarial names. Canonical layout decisions affect StateIds; don't defer that identity consequence without a migration plan.

**Storage:** canonical hashes are independent of compression and physical placement. Prefer one transactional local durability boundary during the first implementation; test segmented blobs, local/object-store caches and bulk bundles only after defining the object-closure and staging/GC protocol. Avoid custom consensus until remote target guarantees require it.

**Sync:** a StateId permits direct hierarchical sparse requests. For full/cold replication, batch and compress object closures, with optional derived reachability/transfer indexes. The protocol should not require a RTT for every tree node, nor ship entire history for every CI job. Both patterns are possible independent of B-versus-A.

**History/merge:** changed-path metadata, ancestry, blame and syntax are *separately derived*, versioned and replaceable. Exact source StateId and accepted provenance are immutable. A semantic validation policy is evidence about an exact candidate, not a promise of automatic correctness.

## 6. Falsification and adoption criteria

Switch to canonical changesets **only** if controlled comparisons show a meaningful whole-workload advantage after including first read, index maintenance, replay tail latency, backup/rebuild and cleanup. Especially test when most durable agent drafts are never inspected, and when most revisions are immediately built.

Switch to causal change graphs **only** if realistic selective-undo/rename/concurrent-history traces show a sufficiently large correctness/ergonomics win with competitive arbitrary-source/materialization and retention costs.

Likewise abandon a particular tree format or physical store if bounded-node, write amplification, crash safety or cold access measurements fail.

No incumbent's metrics demonstrate that AVC is already better. If full workflow studies show only marginal value over tuned Git/Jujutsu and existing agent harnesses, revise project scope rather than marketing ordinary features as unprecedented.

The existing [contract probes](experiments/vcs_contract_probes.py) and [recorded results](experiments/vcs_contract_results.json) demonstrate narrow counterexamples (identical deletion/revert, disjoint-file semantic breakage, external-writer restore race, prepared-object GC race, global-head contention). They are **not** a native VCS implementation or performance benchmark.

## 7. Decision register

| Decision | Current status |
| --- | --- |
| Native general-purpose VCS; Git exchange optional, not a requirement | User requirement |
| Exact immutable tracked source with direct StateId | **Provisional canonical model** |
| Revision DAG with stable logical change identities | Strong working design |
| Separate bounded private recovery and authored history | Strong working design |
| Per-workspace/target roots, conditional shared landing | Strong working design |
| Conservative typed conflict source, operation undo vs content revert | Strong working design |
| Canonical bounded directory-node encoding | **Open pending focused tree test** |
| Source-root vs canonical delta history | **Snapshot-first reference; benchmark remains a gate** |
| Local DB, remote object physical layout, large-file chunking | **Open optimizations, SQLite reference** |
| Causal patch graph for independent selective undo | Distinct conditional alternative |
| Git import/export, pack encoding, named branch equivalence | Not required / future option |
| Agent scheduler, VM snapshot or CI orchestration in core | Out of scope |

The next step is a small independent source-state interpreter and benchmark that compares A/B on identical inputs; the format should not be frozen until the comparison and crash invariants are recorded.
