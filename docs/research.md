# AVC research — decisions and rejected architectures

**Decision review: 2026-10-15.** The [design](design.md) is authoritative; the [verification plan](roadmap.md) defines qualification. This research is an evidence and trade-off record, **not** native AVC performance measurements.

## 1. What matters most

Optimize complete source-development tasks rather than raw commits/s. A general-purpose VCS must work for arbitrary filesystem writers on laptops and VPSs, offline projects, persistent remote workspaces, speculative agent tasks, short-lived microVMs/containers, CI, and hosted large repos.

Two distinctions resolve much of the previous indecision:

- **Private captures are not authored revisions.** An agent can generate thousands of recoverable intermediate edits without creating thousands of permanent shared changes or triggering CI. Optimize capture admission and retention independently of authored source history.
- **Source representation does not determine execution location.** A snapshot-root model can serve checkout-free queries, sparse clients, remote agents and highly distributed caches. A delta-root model can run locally. Hosted scale alone does not select canonical history.

The central safety question is: *what exact bytes can the tool reconstruct and protect after this operation?* A VCS that optimizes writes without proving its declared source state is valid and durable is not better.

## 2. Canonical models evaluated

| Model | Strongest advantage | Main drawback for AVC | Decision |
| --- | --- | --- | --- |
| **A. Canonical persistent State tree plus Revision DAG** | Exact historical source root immediately available; opaque files; subtree reuse; simple source trust boundary | Build persistent tree nodes for each authored revision; canonical codec becomes long-term commitment | **Select** |
| **B. Canonical parent-relative changed-file log plus derived source trees** | Admit valid but rarely viewed revisions without storing each root; derived tree encodings can evolve freely | Must still validate source/preconditions; cold reads can replay long history; subtree moves need careful treatment | Reject as the primary history model; use deltas for capture, efficient transfer and change hints |
| **C. Canonical causal patch/file graph plus materialized source** | Selective causal undo and stronger commutation for supported operations | Infer no unique edit intention from arbitrary filesystem bytes; complex rename/delete/binary/reference-retention rules | Do not impose on general source truth; use specialized editing tools if necessary |

**Why A wins on the specified product:** Most meaningful authored revisions get inspected, tested, merged, transmitted or otherwise require an exact source view. An immutable Merkle tree shares all unchanged subtrees. A source snapshot does not store a complete physical copy on every change. Native subtree moves can relink one immutable subtree without enumerating 100,000 descendants.

**Real disadvantage:** A requires a stable canonical directory representation. AVC therefore *selects* one bounded deterministic Merkle radix structure and accepts future explicit format migration, instead of using a flat map forever or pretending tree encoding is an optional derived index.

**Where B could win:** An unusually large stream of already validated, durable authored revisions that are almost never read. Anthropic's hosted workload is closer to that scenario; an ordinary human or agent completing a task often immediately needs the source to test it. A source-delta record must still have a valid base and a valid resulting namespace. Deferring all validation until someone reads the revision means acknowledging a pending draft, not a complete trustworthy source state.

The [reference model](experiments/source_models_oracle.py) has 14 passing deterministic in-memory checks (including 250 edits compared to a separate flat path model). It verifies that a checked delta interpreter and a snapshot interpreter can reconstruct identical source, and that an invalid delta must not be acknowledged as a coherent revision. **It neither compares real storage performance nor proves final radix/merge semantics.**

**Where C could win:** Systems needing causal selective undo of identical independently authored text and strongly commuting editor operations. [Pijul's change theory](https://pijul.org/manual/theory.html) and [Darcs patch algebra](https://darcs.net/Theory/MergersDocumentation) demonstrate real alternatives. Their strongest guarantees depend on the supported editing algebra. Arbitrary editors, binaries and external file-generating tools don't record such operations. AVC cannot pretend to reconstruct causality from final bytes alone.

## 3. Primary-source lessons without incumbent overfitting

### Git and GitHub

[Git objects](https://git-scm.com/book/en/v2/Git-Internals-Git-Objects), [Git pack layout](https://git-scm.com/docs/gitformat-pack), [checkout-free merge-tree](https://git-scm.com/docs/git-merge-tree) and [commit graph](https://git-scm.com/docs/git-commit-graph) already implement immutable sharing, physical deltas, checkout-free merges, and ancestry/changed-path acceleration. Those properties are *not* AVC inventions.

[GitHub's October 2026 writeup](https://github.blog/engineering/architecture-optimization/building-git-infrastructure-for-agent-scale-development/) describes agent push/CI growth and independently scalable durable storage with Git logical history unchanged. It is a counterexample to arguments that agent growth requires a causal graph or canonical file-delta history.

### Jujutsu

[Jujutsu architecture](https://docs.jj-vcs.dev/latest/technical/architecture/), [operation log](https://docs.jj-vcs.dev/latest/operation-log/) and [first-class conflicts](https://docs.jj-vcs.dev/latest/conflicts/) demonstrate stable logical changes, operation recovery, concurrency and multi-sided symbolic conflicts. It is AVC's primary user-semantics baseline. AVC's proposed benefits must come from the complete recovery/workspace/source-API/integration workflow rather than naming or reimplementing these features.

### Anthropic Scion and Meta Mononoke

[Scion (October 13, 2026)](https://www.anthropic.com/engineering/redesigning-source-control) describes canonical changed-file records, derived trees/history/blame, API-first computation, sparse cached clients and narrow shared ref CAS. It reports more than 30,000 changes/day and around 25 million CI checkouts/day plus large improvements against prior Git-mirror infrastructure. The authors note their *first-parent* changed-file format is a stylistic trunk-workflow choice; their speedups do **not** isolate a canonical-delta-versus-snapshot comparison at equal cache/durability/network cost.

[Mononoke's Bonsai data model](https://github.com/facebook/sapling/blob/main/eden/mononoke/docs/2.1-bonsai-data-model.md), [derived indexes](https://github.com/facebook/sapling/blob/main/eden/mononoke/docs/2.3-derived-data.md), and [Bonsai source type](https://github.com/facebook/sapling/blob/main/eden/mononoke/mononoke_types/src/bonsai_changeset.rs) establish closely related canonical changed-file/derived-data techniques. These are distinct operational deployments but **not independent controlled evidence** for B's universal superiority. Disputes over specific design influence do not establish technical optimality.

**Take the useful principles:** no-checkout APIs, optionally asynchronous blame/path indexes, sparse and lazy working copies, immutable content caches and short ref transactions. Each works with the selected State-root architecture.

### Enroute and Cursor

[Enroute storage](https://github.com/enroute-sh/enroute/blob/main/docs/internals/storage.md), [fetch algorithms](https://github.com/enroute-sh/enroute/blob/main/docs/internals/read-path.md) and [Cursor Git-at-scale](https://cursor.com/blog/git-at-any-scale) demonstrate that physical storage, compaction, distributed read-serving and publication can improve dramatically without altering canonical history. Enroute's dense object numbering, commit segments and Raft log solve its hosted Git workload. **Do not transplant them** as AVC's canonical IDs or mandatory coordination mechanism.

### Persistent directory structures

[Meta's Sharded Map V2](https://github.com/facebook/sapling/blob/main/eden/mononoke/docs/2.3-derived-data.md) uses path-compressed tries, bounded nodes and small inline terminals. This directly supports AVC's native directory map choice.

[Dolt's Prolly Tree fast merge](https://www.dolthub.com/blog/2025-07-16-announcing-fast-merge/) demonstrates both good ordered-map sharing and complications when chunk boundaries move. Prolly is a strong alternative for certain tables and ordered ranges, but hierarchical source names and subtree moves favor deterministic prefix partitioning with bounded small leaves.

### Merge and collaborative editing

[2026 MergirafSemi](https://arxiv.org/abs/2608.11345) documents a genuine tension between reducing apparent conflicts and allowing incorrect automatic combinations. AVC prioritizes preserving uncertain alternatives. [Eg-walker (EuroSys 2025)](https://martin.kleppmann.com/2025/03/30/eg-walker-collaborative-text.html) informs optional causal collaborative text editors, but does not provide arbitrary file tree or binary semantics.

### Local durability

[SQLite synchronous/WAL documentation](https://www.sqlite.org/pragma.html#pragma_synchronous) distinguishes the failure assumptions of FULL and NORMAL durability. A transactional local metadata authority and protected immutable objects reduce new trusted code versus a custom consensus/journal implementation. The protocol must still handle prepared blobs, directory fsync, reader epochs, concurrent GC and backup closure.

## 4. A coherent design, not a feature inventory

AVC has **one authority per type of fact**:

| Fact | Authority | Other representations |
| --- | --- | --- |
| Exact source | Content-addressed State objects | Changed paths and diffs are derived/verified |
| Authored history | Immutable Revision DAG | Stable logical ChangeId and evolution links |
| Unfinished recovery | Protected time-bounded Capture | Cannot claim a complete authored revision unless promoted/validated |
| Working-directory selection | Workspace selection generation | Files on disk are external observations; status is fallible |
| Shared acceptance | Per-target transactional root | Tests/reviews are evidence about exact immutable candidates |
| Build/deployment | External execution/artifact system | AVC may reference their provenance but does not own process state |

The model does not need a permanent authoritative delta log plus an authoritative snapshot log plus an agent-operation log. Private captures may be stored *incrementally* without being a second version of canonical authored source.

A metadata-only or sparse agent can inspect an immutable StateId without obtaining an entire local repository. A full local repo remains self-contained and offline. A short-lived remote agent can commit/pin its exact new source before discarding its VM. These outcomes follow from immutable source IDs, resource-scoped durability and native APIs, **not from adopting Scion's or Enroute's physical encodings**.

## 5. Costs and overturn conditions

Decisions are selected, but engineers should not pretend proven metrics:

- **Snapshots:** pay tree construction when recording authored history. A measured highly speculative workload could motivate a cheaper draft representation; AVC already has captures/pending edits for this.
- **Radix map:** larger per-node overhead and key-prefix processing than carefully paged POS structures in some workloads. Keep the canonical algorithm; tune bounded nodes before stable format release.
- **Typed conflicts:** multiway normalization, merge-base recursion and marker presentation are hard. Preserve all alternatives and fail closed on complexity limits.
- **Metadata in revision IDs:** changing a description can rewrite descendant IDs; stable logical ChangeId helps user identity but does not remove rewrite computation.
- **SQLite authority:** excellent local correctness starting point, not a distributed hosted database. Hosted target/root authority uses its own storage while preserving logical CAS.
- **No Git compatibility:** frees the native model but costs adoption and existing tooling integration. Self-contained native backup/export is mandatory; third-party bridges are elective.

The existing [contract probes](experiments/vcs_contract_probes.py) record wrong merge assumptions, stale filesystem overwrite, staged-object collection and global-head contention. They are counterexamples, not production VCS tests.

**Reconsider a fundamental choice only if:** it violates a specified correctness invariant, makes critical general-purpose workflows unusable, or robust complete-task measurements establish a material advantage for an alternative. Do not re-open design simply because a new article features a different data structure.

Prior incompatible architectural proposals remain accessible through Git history. The present [design](design.md) supersedes them; no parallel "maybe canonical deltas" roadmap remains.


## 6. Adversarial architecture review — October 15, 2026

This is a **static adversarial review of the proposed contracts**, not a claim of proof or benchmark superiority. It forced concrete changes to the authoritative [design](design.md).

| Attack on the original idea | Counterexample | Final v0 response |
| --- | --- | --- |
| "One conflict per path is enough" | Both sides rename one source to different destinations, coupling several paths | Represent conflicting subtree alternatives at their least common ancestor, including the whole-root case |
| "Source conflicts include their origin" | Two histories yield identical unresolved byte alternatives | Hash value-only conflict terms in State; store driver and input RevisionIds in separately owned immutable provenance |
| "Flatten all nested conflicts" | Criss-cross and repeated rebases can cancel meaningfully distinct alternatives | Simplify only proven equal/base cases; preserve nested terms; bounded opaque ConflictGroup DAG if normalization cannot prove safety |
| "An enormous conflict is a single blob" | 10,000 alternatives or chained merges consume unbounded memory | Kind-8 canonical chunked conflict groups with bounded child fanout and complexity errors, no silently lost input |
| "Root can only be a directory" | Root is least common ancestor of two coupled rename conflicts | State root allows a typed unresolved *directory-valued* conflict |
| "Conflict at root hides all source" | Most files are unaffected by a single rename collision | Read paths across all alternatives; return resolved value if every alternative agrees, typed uncertainty otherwise |
| "Merger picks any common ancestor" | Criss-cross history has more than one best merge base | Compute maximal common-ancestor frontier; deterministic memoized virtual-base with conflict preservation and recorded recipe |
| "Same bytes means same causal effect" | Independent identical deletion then revert | State dedup means value equality, not causal selective undo. Preserve origin and allow explicit reapply |
| "Large moves require file-level deltas" | Rename directory with 100,000 files | Immutable child-directory map binding reuse; copy by content SubtreeId instead of pinning dead revision ancestry |
| "Radix choice is enough" | Threshold-crossing map yields different hashes by insertion order | Fixed deterministic maximal-prefix nibble radix split; leaf/branch constraints and byte grammar are part of canonical identity |
| "Every checkout creates a new directory" | Ordinary developers switch revisions frequently in one editor | Restore/new workspace uses strong fresh-directory guarantee; ordinary in-place switch allowed with protected before-images, refused known dirt, and honest external-writer race limitations |
| "The DB transaction protects live files" | Editor writes after scan or keeps a descriptor across rename | Selected immutable root is independent of external observed filesystem; status is time-scoped, in-place exclusion is explicitly limited |
| "Uploaded bytes means agent work is saved" | Worker VM dies after sending unpinned object files | Required protected draft root and whole source dependency closure must be acknowledged before VM deletion |
| "Unchanged diff paths imply valid program" | Agent edits API while another edits caller, with no text conflict | Validate exact integrated State under a declared toolchain/policy, CAS accepted target against observed generation |
| "CAS is one global operation head" | Capture watcher races independent workspace selection | Separate selection, capture and materialization generations; only same named shared target competes logically |
| "Content hash means trusted source" | Attacker guesses a blob ID across tenants or sends malicious paths | Per-tenant auth closure checks, exact path witnesses, symlink-safe materialization and no implicit hooks |
| "Staged object GC is harmless" | New object is collected between prepare and root publication | Stage pin, durable object placement, root transaction and reader/GC fences |
| "Offline and cloud demand different canonical stores" | Same source created locally then queried remotely | One native content ID model; full, partial and service clients differ in transport and retained closure, not source semantics |

### Research that sharpened this review

- [Dolt, September 8, 2026: Can Radix Trees Replace Prolly Trees?](https://www.dolthub.com/blog/2026-09-08-can-radix-trees-replace-prolly-trees/) distinguishes radix prefix-partition advantages from prolly's balanced pages and comparator flexibility. AVC keys are **raw byte names**, making radix ordering natural, but this does not prove best latency for every workload. v0 chooses one deterministic radix implementation so source identity is implementable; physical paging remains independent.
- [Jujutsu's technical conflict model](https://github.com/jj-vcs/jj/blob/main/docs/technical/conflicts.md) explains nested merge terms and the deliberate **lossiness** of its same-change rule in some repeated rebase scenarios. AVC explicitly keeps source-value equality separate from authorship/causal undo, and refuses arbitrary cancellation of nested unresolved conflicts.
- [Git merge-tree](https://git-scm.com/docs/git-merge-tree) demonstrates checkout-free recursive merge bases and warns that many-to-many rename/directory conflicts cannot be reduced to one conflicted-file marker. AVC models coupled source regions rather than using markers as authoritative files.
- [Microsoft Research YoloFS, April 2026](https://www.microsoft.com/en-us/research/publication/dont-let-ai-agents-yolo-your-files-shifting-information-and-control-to-filesystems-for-agent-safety-and-autonomy/) studies agent destructive filesystem effects. AVC is not an agent filesystem, but its optional protected draft/staging and explicit writer control boundaries let a VM/filesystem security layer integrate without making process state canonical source.

### Findings and limit

The review reinforces the **single source State authority**, stable Revision/ChangeId split, independently retained recoveries, and target-scoped publication. It also exposes an ergonomics tradeoff that the previous spec buried: requiring a fresh directory for *every* human branch switch is not acceptable in a general-purpose VCS, while claiming no data loss under arbitrary concurrent external writes is physically impossible. The [design](design.md) now offers both an ordinary guarded in-place switch and a separately stronger fresh-directory restore, with different stated guarantees.

**Still requiring proof** rather than architectural re-voting: canonical byte vectors and history-independent radix normalization, structured conflict algebra under deep repeated merges and criss-cross, safe in-place-switch recovery following partial OS writes, durable SQLite/external-object publication under real fault injection, and actual platform/network performance. These are specific correctness/qualification obligations in [roadmap](roadmap.md). None justifies an automatic wholesale migration to a different VCS architecture.
