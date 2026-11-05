# AVC — adversarial architecture decisions and evidence

**Reconciled 2026-11-05.** This is a **decision and falsification record**, not a second source/specification. Authoritative logic: [design](design.md), [canonical format](format.md), [storage](storage.md), [native protocol](sync.md), [Git bridge](git.md), [UX](ux.md), and [roadmap](roadmap.md). The extensive older research corpus, independent prototypes and their full measurements are [preserved in Git history](https://github.com/nijaru/avc/blob/af66a63898f383b596e4828199dc625a80e28a72/docs/research.md); the historical experiment source is [also preserved](https://github.com/nijaru/avc/tree/af66a63898f383b596e4828199dc625a80e28a72/docs/experiments). **Do not reinstate old code/fixtures as normative simply because they existed.**

Sections 1–12 synthesize upstream documentation and earlier bounded experiments, not new AVC performance or durability evidence. Section 13 records the first new implementation checks and their limits.

## 1. Strongest alternative: don't build a new VCS at all

Git + Jujutsu + GitButler + an agent harness already provides immutable Git source, stable logical change IDs, operation history, change stacks, flexible UI and broad adoption. [Jujutsu architecture](https://docs.jj-vcs.dev/latest/technical/architecture/) deliberately separates repository data, operation storage, working copy and presentation; [GitButler parallel branches](https://docs.gitbutler.com/features/branch-management/virtual-branches) supports several applied branches in one directory. A Git backend would reduce protocol, storage, migration and ecosystem risk enormously.

**Reason to pursue AVC nevertheless:** the user explicitly wants a genuinely native VCS whose source object model, on-demand fetch, conditional agent edits, structured conflict semantics and offline recovery do not inherit Git's format or client requirements. This is a *product bet*, not a proof of intrinsic superiority. AVC must win at complete source-development workflows and trustworthiness, not novelty or microbenchmarks. **Stop/reassess** if the native implementation cannot deliver meaningful practical gains over Git plus mature clients.

## 2. Scion/Mononoke changesets versus persistent Merkle snapshots

[Anthropic Scion (Oct 13, 2026)](https://www.anthropic.com/research/redesigning-source-control) records canonical first-parent changed-file sets and derives manifests, ancestry/path/blame indexes on servers. [Mononoke Bonsai](https://github.com/facebook/sapling/blob/main/eden/mononoke/docs/2.1-bonsai-data-model.md) applies a similar separation at large scale. These are powerful for extremely high-rate, always-online commits with infrequently materialized source. **Their vendor-measured speedups mix caching, virtual workspaces, distributed metadata, traffic and compute; they are not controlled proof that canonical changesets beat Merkle roots.**

**Selected for AVC:** canonical snapshot roots built **incrementally** via structural sharing; a normal one-file edit need not rebuild or copy the entire repository tree. Direct exact source identity and offline readiness matter on ordinary local and ephemeral workloads. Store changed-path summaries as derived authenticated hints, not a second canonical interpretation.

**Falsification:** if a correct incremental Merkle implementation cannot achieve acceptable edit/clone/query performance while a changeset-first model demonstrably can with comparable offline proof/custody, revisit authority before v1 release.

## 3. Pijul/Darcs causal changes versus exact source

[Pijul theory](https://pijul.org/manual/theory.html) and [Darcs patch algebra](https://darcs.net/Theory/MergersDocumentation) offer a credible stronger model for causal patch composition, selective undo and commuting independent changes. It would be tempting to make such changes the universal repository history for simultaneous coding agents.

**Corrected objection:** ordinary filesystem writes do not reveal unique *user intent*, but this does **not** prevent constructing a causal change graph. Pijul explicitly compares its ordinary working copy to its recorded pristine state and records graph changes from the observed diff; vertices retain identities supplied by their introducing change. The real trade-off is maintaining correct dependency/line/path identities and representing binary, rename, delete and mode semantics through repeated composition. AVC chooses exact source-tree roots for general offline custody and direct source querying while allowing validated structured edit descriptions **outside source identity** when tools supply useful intent. Pijul is a serious change-model alternative, not disqualified by external editors. [Pijul getting started](https://pijul.org/manual/getting_started.html), [recording changes](https://pijul.org/manual/workflows/splitting_and_combining_changes.html) and [theory](https://pijul.org/manual/theory).

**Falsification:** a bounded causal prototype wins whole-task repeated agent merge/rebase cases with arbitrary-file/path correctness and substantially simpler retention than AVC's source-root model.

## 4. Conflict algebra: deliberately change the November 3 preference

The older AVC draft stored **nested opaque conflict alternatives** and used an all-optimal-alignment text algorithm to minimize false-clean outcomes. This preserved uncertainty but generated extra conflicts and a depth budget in repeated history changes. The earlier recorded 64-triple historical sample showed **54 clean/10 conflicts** with its refined conservative driver versus **58 clean/6 conflicts** for Git and Jujutsu. Inputs came from only 14 sampled Git merges and are **not a semantic correctness oracle**. [Original cohort and limits](https://github.com/nijaru/avc/blob/af66a63898f383b596e4828199dc625a80e28a72/docs/research.md#21-sparse-ambiguity-without-a-dense-fallback--october-29-2026).

[Jujutsu's technical conflict model](https://docs.jj-vcs.dev/latest/technical/conflicts/) represents tree expressions of the form `A+(C-B)`, flattens nested expressions and cancels equal opposite terms. This prevents an unresolved conflict from growing into nested conflicts during repeated rebases. Its **same-change rule is explicitly lossy** under later rebasing. AVC adopts the valuable algebraic normalization and refuses **additional lossy cancellation of nonidentical unresolved terms**. It nevertheless uses conventional `L=R` clean resolution. This can lose latent *edit intent* across later rebases even though final source bytes remain exact; v1 deliberately discloses emptied changes and retains old immutable revisions for scoped undo, rather than claiming fully invertible history algebra. That trade-off is stated in [format](format.md).

**Selected in [format](format.md):** flat signed *path-value* terms with exact cancellation, typed binary/type/absence alternatives and bounded expression size. Source bytes do not include hidden Git-style marker text. When input representation/merge work cannot finish, pin inputs and return pending, not fabricated conflict. A practical deterministic versioned text merge may cleanly combine disjoint hunks; its correctness and burden require real tests. The new expression encoding is a **design hypothesis awaiting executable verification**, not a proven fully associative universal patch algebra.

**Falsification:** path-local signed terms cannot compose real directory moves, delete-vs-change and nested resolution without erasing alternatives, or hit term limits often under ordinary restacking. Replace the unreleased conflict format coherently, possibly with canonical whole-tree merge expressions, *before* use with important data.

## 5. Directory map: choose now; tune only with evidence

A prior controlled surrogate on one pinned git/git source snapshot tested 4,582 regular/symlink bindings, 64 present/missing paths, and three map layouts:

| Shape | Unique map objects | Map bytes | Median cold present-path bytes | New map bytes after one file edit |
| --- | ---: | ---: | ---: | ---: |
| Flat directory entries | 222 | 260,679 | 94,525 | 98,831 |
| Packed radix, old 4 KiB limit | 479 | 260,437 | 4,168 | 6,795 |
| One-key radix | 7,098 | 484,998 | 1,951 | 1,639 |

These were **Python/surrogate structures using SHA-256 refs**, not native BLAKE3/SQLite/segment timing or realistic cache results. More compact reads sometimes required more object requests. Move validation still traversed many descendants without a trusted bound summary. [Full old study](https://github.com/nijaru/avc/blob/af66a63898f383b596e4828199dc625a80e28a72/docs/research.md#20-public-history-and-map-access-costs--october-29-2026).

**Selected:** per-directory path-compressed nibble radix with **32-entry leaves**, a simple count-normalization rule and persistent path-copy updates. It is not the old 4 KiB leaf format. A deterministic chunked ordered map could improve range scans and packing, but stable chunk boundaries and edit churn introduce their own complexity. A single controlled Rust comparison before stable format release can falsify the 32-entry choice; it is not grounds for another whole-design loop.

## 6. Storage: the strongest objection is crash safety

[SQLite WAL/synchronous documentation](https://www.sqlite.org/pragma.html#pragma_synchronous) supports simple reliable local transactions, especially under FULL durability. A SQLite-only blob store would minimize trusted code and cross-file recovery states. On the other hand, immutable segment storage avoids writing large compressed objects into SQLite WAL pages, supports streaming, compaction, and cheap transfers. [Zstandard specification](https://github.com/facebook/zstd/blob/dev/doc/zstd_compression_format.md) defines portable compressed frames/dictionaries; this does **not** prove per-object compression beats Git's delta-packed blobs.

**Selected:** SQLite metadata/ref authority plus durably sealed immutable segments for payload, with small-inline storage as a physical optimization. [Storage contract](storage.md) fixes ordering, crash cases, reader leases, GC and independent export. A SQLite-only control is a **qualification reference**. If hybrid durability cannot be proven under representative fault tests and the actual performance gains are small, replace physical storage with SQLite-only **without changing canonical ObjectIds**.

[BLAKE3 specification](https://github.com/BLAKE3-team/BLAKE3-specs) supports fast parallel hashing with domain separation; it does not remove the need for verification, authentication, caching, bounded decompression or careful sync. Modern primitives should improve whole-task costs, not be used as marketing substitutes for measurement.

## 7. Native sparse clones versus foreign Git partial data

[Git partial-clone documentation](https://git-scm.com/docs/partial-clone) explicitly distinguishes intentionally promised missing objects from corruption and warns about inefficient per-object demand fetch. Native AVC sparse clients may know the **authenticated full BLAKE3 Merkle root** and construct a conditional update with exact path proofs/sibling commitments while only selected blobs are local. The full holder validates/publishes the complete closure.

An initially filtered **Git** clone, however, knows SHA object IDs for unfetched blobs but cannot derive the BLAKE3 hashes of unknown content. Consequently the prior idea of transparent "native sparse import from arbitrary Git" was incorrect without an additional foreign representation. [Git adapter](git.md) explicitly offers (a) complete selected-tip import with foreign ancestry frontier, or (b) a separate **foreign partial view** capable of Git-relative reads/patches/pushes without pretending to have a native source root. Native conversion requires hydrating current source bytes.

A future mixed-ID canonical source model might eliminate that conversion barrier, but sacrifices the simple invariant that identical canonical file bytes have one native identity regardless of origin. It is rejected until compelling evidence outweighs permanent identity/backup complexity.

## 8. Canto, agents and product boundary

GitButler-style composed local change stacks are an ergonomic **derived view**, not another canonical state store. [Sapling absorb](https://sapling-scm.com/docs/commands/absorb/) is a compelling workflow for reattributing unambiguous later edits, but its guesses must not become source truth. Separate writable agents remain safer under unknown collisions; shared directories may be useful for cooperating agents with known file/runtime interference. AVC's exact source APIs should make either possible.

AVC is **not** Canto's agent orchestrator, a task tracker, a CRDT editor, CI/build execution, code review hosting, or a VM snapshot manager. Keeping that boundary avoids inventing retention/disclosure invariants for data the VCS should not own.

## 9. Why obsolete code and studies were retired

The former `src/objects.rs`, `src/merkle.rs`, `src/verify.rs` and `src/lib.rs` implemented the **superseded** state-wrapped 4 KiB map format with whole-directory construction and full-map verification. The ~50 Python/Node/Rust study fixtures, old golden vectors and CI asserted obsolete source identities, conflict algorithms, or models not usable as production guarantees. Running them after this redesign would encourage future agents to conform to the wrong format. They were removed from the **active tree**, but their exact source/results remain available in the pinned Git commit linked at the top. They may be deliberately consulted to recreate **new independent conformance checks**, but should not be copied wholesale.

The code reset is intentional, authorized and not a claim that no earlier research was useful. The November 5 implementation starts from **current contracts**, with fresh independent vectors and real failure-path checks; it does not revive the retired codec.

## Git backend decision: preserve adoption without two source authorities

**November 4, 2026 choice:** permanent Git import/export plus eventual Git smart-protocol serving from native hosting, **not** a separately maintained Git-backed AVC engine. The strongest alternative is Jujutsu's existing Git backend: immediate mature Git transport/storage and unmodified ecosystem workflows, at the cost of foreign metadata/identity constraints. Mononoke demonstrates a canonical non-Git backend with derived Git-facing representations, but it is a high-complexity system, **not** a ready-to-copy template for a small local-first VCS.

The crucial operational failure would be two independently writable repositories claiming the same branch. The chosen policy makes the Git-visible ref **one guarded native ref** whose exact current tip is exportable. The frontend derives Git object OIDs/packfiles and conditionally translates Git pushes, but never mutates a shadow Git history as an alternate authority. Native-only refs may contain unexportable source and must remain distinct. **This is a costly compatibility service**, so implement it only after native hosting and ordinary Git remote bridge demonstrate adoption needs. The user can still collaborate through external GitHub/GitLab without this service.

**Falsification:** if Git hosting parity and mixed-client consistency require a materially more complex mapping/retention layer than users value, retain Git remote import/export indefinitely and **defer** the Git-hosting frontend. Do not sacrifice source identity or create a second backend merely to satisfy a speculative hosting milestone.

## 10. November 4 competitor steelman after the one-native-backend decision

**Scope:** contemporary official documentation and vendor announcements, reviewed November 4, 2026. This is an **independent competitive assessment, not an AVC benchmark or a release guarantee**. It updates the earlier conclusion that Git interoperability can be long-lived through one native authority plus conversion layers. The normative interface contract remains [git.md](git.md), not this discussion.

### What users can already accomplish without AVC

| Alternative | Strongest product/technical case | Unproven opportunity for AVC |
| --- | --- | --- |
| **Git + Jujutsu** | Jujutsu's production-oriented Git backend delivers stable logical changes, first-class conflicts, operation-log recovery, automatic working-copy snapshots and interop with existing Git remotes. Developers can keep all Git tools and hosted workflows today. | A truly integrated **checkout-free conditional source API and native partial-source custody** with lower overhead, fewer foreign-layer compromises, and predictable agent integration. Must benchmark complete tasks against modern `jj`, not old Git alone. |
| **Sapling + Mononoke + EdenFS** | Source control for very large monorepos, tiny working sets, source indexing, and a single canonical Bonsai authority with derived Git/Sapling/Mercurial formats. Direct empirical precedent for **one engine, multiple VCS protocols**. | Local-first portable deployment without the server/virtual-filesystem infrastructure and a source model optimized for direct immutable tree queries. No proof that AVC's snapshot authority outperforms Mononoke's changesets in high-throughput hosting. |
| **Pijul / Darcs** | Causal change dependencies and commuting independent changes offer partial clones and repeated independent integration without rebasing away identities; first-class conflicts are part of the history model. | Complexity of preserving file/line identity across binaries, modes, renames and repeated conflict composition, plus unfamiliar Git adoption; Pijul **does** record edits from ordinary working copies. **Major technical threat:** if Pijul's model yields materially fewer incorrect combinations or repeated merge conflicts in agent workloads, AVC's snapshot-plus-diff representation is the wrong trade-off for those cases. |
| **GitButler + Graphite** | Applied parallel changes in one workspace, hunk ownership, stacked PRs and existing GitHub workflow; no custom native repository migration required. | Exact immutable composed source values, safe independent agent conditional work, and reliable protected unfinished state. Merely duplicating stack UX is not enough. |
| **DeltaDB** | Causal character-level deltas, continuous agent-session history, replication and worktree materialization between Git commits; Git remains the shared authored state. | Lower-cost arbitrary-file source custody and capability without mandatory Git or continuous conversation capture. A source-only VCS loses Delta's fine-grained intention/provenance UX; integrations must remain possible. |
| **Entire** | Git-compatible distributed mirrors aimed explicitly at agent-scale read/push traffic; integrated agent checkpoints, explainable blame and review. Direct competition for "agent Git backend" and remote context services. | Native transactional source work without Git object/partial-clone constraints. **Do not compete on raw Git hosting throughput alone.** Their published performance claims are vendor-specific load tests, not AVC baseline targets. |
| **OpenAI Pando and GitHub's new infrastructure** | Mature Git and ecosystem compatibility alongside independent per-ref write coordination, immutable reusable pack transfers, compaction, regional caches and elastic Git serving. | New **source semantics**, no-checkout queries and safer multi-agent edit/integration, rather than another faster Git transport layer. Protocol compatibility alone is not differentiation. |
| **Fossil** | Reliable portable SQLite repository and independently verified atomic source transactions, without dual physical-store complexity. | AVC must justify its SQLite + sealed-segment durability/GC/backup burden against a simpler single-database baseline. Native physical speed alone is not a safety argument. |

**Current primary references:**
[Jujutsu architecture](https://www.jj-vcs.dev/latest/technical/architecture/),
[Jujutsu Git backend details](https://www.jj-vcs.dev/latest/technical/architecture/#gitbackend),
[Mononoke derived data](https://github.com/facebook/sapling/blob/main/eden/mononoke/docs/2.3-derived-data.md),
[Mononoke VCS interoperability](https://github.com/facebook/sapling/blob/main/eden/mononoke/docs/README.md),
[Pijul change theory and partial clones](https://pijul.org/manual/why_pijul.html),
[GitButler parallel branches](https://docs.gitbutler.com/parallel-branches),
[Graphite stacked PR workflow](https://www.graphite.com/docs/cli-quick-start),
[DeltaDB](https://delta.dev/deltadb),
[Entire distributed Git launch (July 8, 2026)](https://entire.com/news/entire-launches-distributed-git-network-for-the-agent-era),
[Pando (November 2, 2026)](https://openai.com/index/scaling-git-at-openai/),
[GitHub agent-scale architecture (October 2026)](https://github.blog/engineering/building-git-infrastructure-for-agent-scale-development/),
[Fossil SQLite guarantees](https://fossil-scm.org/home/doc/trunk/www/index.wiki).

### Why the new Git frontend is not an immediate feature

Mononoke confirms that a canonical non-Git source model can serve Git through derived objects and mappings. **It does not make this inexpensive.** Git protocol push/fetch semantics, foreign IDs, ancestry mapping, ref leases, signature/tag fidelity, shallow promises, object reachability and long-lived cache retention are substantial additional maintained surface area.

**Decision maintained:** one native authority, enduring Git remote import/export, later optional Git smart frontend. A Git-visible ref is guarded by exportability and a single native ref generation. If allowing native conflicts or empty directories makes common shared branches impossible to update, users must have a clear native-only path and a *deliberate* representable integration candidate; this policy requires UX trials before a Git frontend launches. The strongest alternative remains simply continuing to rely on GitHub/GitLab for collaboration without building an AVC Git server.

A second Git-backed AVC engine would remove some migration friction but create two implementation truth paths and feature differences. That is not justified unless substantial actual adoption demonstrates native conversion is the overwhelming bottleneck and no single-authority bridge can solve it at acceptable cost.

### The four biggest AVC disadvantages

1. **Ecosystem and trust:** Git has decades of interoperability, integrations and production failure testing. The AVC format, conflict algebra and cross-store durability are unimplemented and may contain subtle data-loss cases. No presumed correctness advantage until faults are verified.
2. **Migration cliff:** a normal Git import needs the complete selected current source to compute native hashes; a filtered Git foreign view retains Git identities but cannot use AVC's full native authored feature set. This can neutralize AVC's biggest value proposition exactly for the largest monorepos.
3. **Causal composition:** Pijul/DeltaDB retain edit intent unavailable from plain snapshots; AVC's signed conflict expressions preserve source alternatives but cannot guarantee future rebases recover latent intent discarded by a clean merge. This matters when many agents repeatedly restack.
4. **Scope/maintenance:** source engine, portable safe filesystem operations, hybrid storage, Git conversion, native partial protocol, sophisticated UX and optional Git hosting is a formidable maintenance surface. Pando/Entire/GitHub already invest heavily in scalable Git hosting; AVC should not try to equal all of them before demonstrating one compelling user workflow.

### Falsifiable product wager and benchmark gate

**Proposed first compelling use case:** independently working human/agents on a **very large source tree** can inspect exact source and produce multiple safe candidate changes, review a composed result, recover interrupted work and integrate through Git **with less setup/copying, fewer surprise conflicts and less risk of losing edits** than Git plus Jujutsu/GitButler/agent tooling. This is a *hypothesis*, not a fact.

A credible head-to-head trial needs:
- A small ordinary Git project (adoption/push/PR), a deep-history monorepo with very large current source, generated outputs and two isolated/cooperating agents, and a binary/path conflict-heavy project.
- Identical agent actions and comparable isolation/network/CPU/disk resources for `git+jj`, `GitButler`, and AVC; include Git foreign partial mode as its own outcome, not mislabel it fully native.
- Measure **time to first useful edit**, total wall-clock task completion and p95, downloaded bytes, RAM/SSD churn, recoverable source after failures, manual merge corrections, Git parity and complexity to deploy/maintain.
- Explicit fault cases: aborted source capture, competing edits to same file, clean but semantically incorrect merge, simultaneous native/Git publication, missing promisor data, failed backup and recover after crash.
- Evaluate whether AVC wins *more than one complete high-value workflow* rather than only format microbenchmarks. **If Git + jj + current agent tools are comparable while AVC remains harder to adopt or recover, the independent native system is not yet earning its cost.**

This criterion should drive local implementation qualification and prioritization. Avoid optimizing enormous Git hosting throughput before the **local agent-source transaction and Git round-trip** work provides visible differentiation.

### Earlier Stage 1A proposal (superseded by §12 evidence-first disposition)

**Decision after the competitive steelman, November 4, 2026:** Change *priority and validation*, not the one-native-backend decision.

- **Test the intended product advantage before building hosting.** The previous plan implemented full durable storage and Git onboarding before checkout-free source transactions, and postponed derived multi-change composition until Stage 6. That risks finishing an expensive new VCS before testing its purported advantage. [Stage 1A](roadmap.md#stage-1a-research-resolved) now adds a bounded, disposable, pure-engine two-agent conditional-edit/composition test immediately after persistent source editing. The full UI and hosted protocol remain later.
- **Make causal-change failure a pre-freeze counterexample.** Snapshot-root plus signed conflicts is not necessarily as useful as Pijul's commuting changes in repeated independent agent restacks, especially when a clean merge drops latent edit intent. Test overlapping changes, move/delete/rename, conflict resolution followed by rebase and change-order independence **before** treating v1 conflict bytes as committed. No automatic switch to Pijul or dual canonical patch store.
- **Move the large-Git conversion cliff into first-stage validation.** The native BLAKE3 model requires full current-source bytes for foreign Git conversion. The foreign partial mode avoids this, but leaves native history and checkout-free AVC semantics unavailable. Use real large filtered Git tasks to quantify whether that limitation blocks adoption. If it does, reconsider foreign overlay/adaptor ergonomics with one canonical authority, not silently insert SHA references into native source identities.

These are actionable tests with **stop/reconsideration conditions**, not claims of observed wins. The selected source/ID architecture, permanent Git import/export, and optional future Git-protocol frontend remain unchanged unless measured results falsify them.

## 11. How to overturn a decision

An overturn requires one specific violated invariant, an essential workflow that cannot be supported, or a controlled material end-to-end advantage. State the exact input/conditions, competing contract, data-loss/security implications and migration impact; then update **one authoritative owner** and its dependent implementation. Avoid indefinite algorithm menus, unversioned format changes after release, and marketing claims of globally optimal performance. The [roadmap](roadmap.md) now begins implementation, not further architecture exploration.

## 12. Evidence-first resolution

**Finding:** existing implementation contracts and published evaluations resolve much of the former Stage 1A feasibility/comparison work **without an AVC benchmark or a Pijul reimplementation**. Research first, then write focused tests only for semantics specific to AVC's unreleased codec, conflict algebra, durability or foreign conversion. This section supersedes the earlier suggestion to demonstrate **novelty** of checkout-free operations or benchmark entire competitor systems before writing a usable AVC.

### Checkout-free source operations: solved elsewhere

- [Git `merge-tree --write-tree`](https://git-scm.com/docs/git-merge-tree) merges, handles rename/file-directory conflicts and recursive bases **without reading/writing the worktree or index**.
- [GitHub's Git Trees API](https://docs.github.com/en/rest/git/trees) creates updated immutable trees on an existing `base_tree`, followed by an independent commit and ref update. This can be used without a local checkout.
- [Mononoke's derived data](https://github.com/facebook/sapling/blob/main/eden/mononoke/docs/2.3-derived-data.md) offers read APIs, cached representations and VCS translations from one source of truth.

**Disposition:** stop claiming checkout-free edits or immutable candidate trees are unique. AVC's proposed combination of **authenticated sparse Merkle proofs, typed expected-value updates, isolated candidate composition, source custody and guarded publication** is a concrete integration contract, not a previously unsolved concept. No broad feasibility benchmark is needed. Once the pure engine exists, test only exact stale-path witnesses, disjoint/overlapping candidate composition, type/delete/rename cases and incomplete proofs.

### Causal changes: proven alternative, but not a decisive general replacement

- [Pijul's theory](https://pijul.org/manual/theory) preserves per-change line identities and graph edges; independently commuting changes can be applied in different orders. [Pijul records from normal working directories](https://pijul.org/manual/getting_started.html). The old claim that arbitrary editors prevent constructing graph changes was incorrect.
- [Pijul conflicts](https://pijul.org/manual/conflicts.html) model unresolved contradictory order/deletion/rename relationships; commutation of compatible changes is **not semantic correctness of the resulting program**.
- [Jujutsu technical conflicts](https://docs.jj-vcs.dev/latest/technical/conflicts/) document normalized tree expressions and exactly why default **same-change** resolution loses latent edit intent under a later rebase; [Jujutsu configuration](https://github.com/jj-vcs/jj/blob/main/docs/config.md) offers `same-change = "keep"` as a conservative alternative.
- [Eg-walker (EuroSys 2025)](https://martin.kleppmann.com/2025/04/02/eg-walker-collaborative-text.html) has an artifact and comparative benchmarks proving efficient causal collaboration for **text**, not arbitrary whole-source directory/binary/history semantics.

**Disposition:** keep snapshot roots as the source authority for now. Preserve exact *known* structured operation/dependency witnesses when provided, without promising graph-style commutation from every filesystem diff. Do **not** build a Pijul-like alternative store to decide this. Before freezing the signed-conflict format, execute a **small counterexample corpus**: identical-change rebase forward/back, different-order independent hunks, edit/delete and move/edit, conflict resolution followed by rebase, and nested directory conflict. The success condition is honest source/intent retention and explicit ambiguity, **not** matching Pijul's patch identities. A failure may revise AVC's unreleased conflict semantics.

### Merge behavior: peer-reviewed benchmarks already exist

- [Evaluation of Version Control Merge Tools, ASE 2024](https://arxiv.org/abs/2410.09934) tested **16 merge algorithms** and penalized **incorrect clean merges**, not just the number of conflicts. The [replication framework](https://github.com/benedikt-schesch/AST-Merging-Evaluation) contains **6,045 real-world merges** and has been extended for additional algorithms. It is substantially better coverage than AVC's old biased 64-triple sample; its Java/project/test assumptions still constrain generalization.
- [AI Agent Pull Requests on GitHub (July 2026)](https://arxiv.org/abs/2607.04697) mined **33,596 PRs from 2,807 repositories** and replayed **747 coactive pairs**. Reported textual conflicts were **41.7% across-agent versus 19.8% within-agent** in that replay cohort; cross-agent pairs were relatively rare and cannot yield an unconditional population rate. Almost **42% of observed conflicts were structural** (modify/delete or add/add). These are Git/textual conflict signals, not counts of semantic bugs.
- [Git's modern `merge-tree`](https://git-scm.com/docs/git-merge-tree) is already a strong real-world baseline including recursive merge bases. A simple documented `AmbiguousMergeBase` refusal may still be a valid conservative AVC v1 choice, but it is an explicit **product/UX disadvantage**, not an advance over Git.

**Disposition:** use existing public datasets and methods for the **text/structure merge-driver qualification after implementation**, not a bespoke benchmark or new competing history engine. Compare correct clean merges (using available tests/oracles), incorrectly clean output, manual conflicts and resources. Retain small custom test cases only where the published corpora do not exercise typed binary/directory/conditional operations.

### Git partial clones: initial default policy need not be rediscovered

- [Git's partial-clone design](https://git-scm.com/docs/partial-clone) distinguishes missing promised blobs/trees from missing revisions (shallow history). [GitHub's data-driven study](https://github.blog/open-source/git/git-clone-a-data-driven-study-on-cloning-behaviors/) compared full, shallow, blobless and treeless clones across several repositories. Its synthetic test concluded shallow **initial** clone suits disposable CI, repeated **shallow fetches** are expensive, and blobless + sparse checkout is useful for long-lived scoped work. It is workload-specific, not a native AVC performance proof.
- Native BLAKE3 object IDs cannot be deduced from missing foreign Git SHA objects: conversion must read bytes **somewhere**. It need **not** happen on every client. An **optional shared Git→AVC conversion/cache service** could ingest and verify the full selected Git tip once, compute an authenticated native root, then serve selective native source to multiple thin AVC clients. Mononoke's [derived representations and mappings](https://github.com/facebook/sapling/blob/main/eden/mononoke/docs/2.3-derived-data.md) are architectural evidence for amortizing translation, **not** evidence that AVC's exact gateway is implemented or cryptographically proves Git/native equivalence to a client without reading all bytes.

**Disposition:** existing Git docs settle clone-mode trade-offs sufficiently to choose ordinary policies. No need to benchmark depth-one versus blobless in the absence of an AVC implementation. AVC-specific follow-up is to verify exact Git foreign/import mapping, measure one representative **full tip conversion** versus foreign partial mode, and determine whether a **cached remote conversion service** is worth the hosted complexity when demand exists. Native clients must never mislabel unverified foreign content as locally complete.

### What genuinely still needs AVC tests

**Essential:** exact v1 object/typed conflict conformance and property counterexamples; incremental radix edit/verification cost; crash-safe SQLite+segment admission/GC and backup; foreign Git round-trip correctness; native Merkle proof authorization/closure handling; one realistic end-to-end parallel-source task compared with mature Git+jj once the AVC task actually works.

**Not essential as new tests:** proving that checkout-free source edits exist; replaying hundreds of thousands of existing merges in a new custom harness; implementing Pijul's entire causal algebra only to compare; repeating GitHub's generic shallow/partial clone study; measuring a nonfunctional AVC against feature-complete competitors.

**Remaining uncertainty:** no controlled published study was found that compares **AVC's exact chosen signed-conflict + snapshot-root representation** against Pijul under equal workload, implementation quality and durability. We should not invent such a result. Its *specific* correctness needs independent implementation tests, while broad architectural priority can be decided from existing research.

## 13. First native qualification and demonstrated corrections (November 5, 2026)

**Implemented, not frozen:** the new Rust codec and separately authored Python encoder/decoder agree on **35 canonical objects / 23 entry points** and reject **50 malformed corpora**. Neither imports the old v0 implementation or vectors. Rust-generated 400-binding maps also pass the Python full contextual decoder and match its independent full-map builder. A separate Rust rebuild oracle checks 2,500 deterministic incremental edit steps, split/collapse, prefix terminals, long names, relocation bounds and exact old-value witnesses. These are bounded executable checks, not a universal format proof.

An adversarial review exposed a **real false-clean cancellation** in the draft: given unresolved base `C = X + Y − Absent`, independently resolving to X and Y computes `X + Y − C = Absent`. A directory version resurrects the removed base directory even though neither side selected it. The new regression failed before correction. The chosen correction is a typed **UnresolvedBase refusal after outer equality/base rules**, not an invented resolved baseline or another nested conflict grammar. The trade-off is explicit manual handling of divergent resolutions. This differs from the already documented identical-clean-change intent loss, which remains a disclosed snapshot limitation.

Two related corrections now have executable tests and one normative owner in [format.md](format.md): root conflicts forbid Absent on **both signs** (empty source is a map), and conflicting file content retains the independently resolved executable bit on every derived signed File term. The latter regression also failed before correction. Only previously admitted invalid root-conflict examples were removed/repinned; no released bytes or compatibility commitments exist. Region-0/path versus region-1/root placement is checked through owning references, not just standalone object hashes.

Recorded relocation composition uses actual validated operation witnesses, not inferred move authority from equal hashes. Interacting move/edit, divergent-move or move/delete candidates conservatively retain **whole-source signed alternatives**; ordinary disjoint edits still compose. The initial implementation deliberately does not guess a remap or split correlated resolution across independent paths. Snapshot-only merge remains path-based and cannot claim unavailable move intent. This can impose whole-root resolution/review cost; a narrower remapper is future work, not proof that arbitrary delete/add pairs were renames.

The text driver now pins patience/Myers tie-breaking, retained CR/LF/CRLF tokenization and insertion-boundary rules. Small exhaustive checks independently compare Myers shortest paths, reconstruct input pairs and check side symmetry including budget refusal. Work exhaustion returns Pending rather than selecting a different alignment or manufacturing a semantic conflict. Passing these cases does not replace the established merge evaluation corpus or establish program correctness.

A local Git fixture exercises a **real blobless/no-checkout clone**, changes a Git tree while reusing an unfetched foreign Blob OID, and confirms that no native source root or working files were fabricated. This is the narrow Stage 1A boundary check, not a Git adapter, large-Git conversion measurement or new clone-policy benchmark. Existing Git/Pijul/Jujutsu research still supplies the broader prior-art evidence.

### Pure-engine cost probe, not a VCS speed claim

`cargo run --release --locked --example source_cost -- 10000 [shared]` constructs one 10,000-binding directory, changes one file, checks the exact one-path diff, then independently re-verifies the result. On an Apple M3 Max, arm64 macOS 26.7.2, Rust 1.99.1, using the source engine at commit `44f1b2f`, two preliminary warm-process invocations per shape measured:

| Names | Build | One edit | Merkle diff | Full verify | New objects / canonical payload bytes |
| --- | --- | --- | --- | --- | --- |
| short common prefix | 65.5–71.5 ms | 11.8–13.6 µs | 18.9–20.3 µs | 0.94–1.01 ms | 5 / 1,417 |
| 220-byte common prefix | 72.2–92.3 ms | 13.0–13.5 µs | 142–146 µs | 2.98–3.21 ms | 5 / 1,637 |

These timings exclude disk/SQLite, cloning, cold object acquisition, filesystem capture and durability. All old in-memory construction objects remain retained during the probe; it is not an allocation/RSS or sustained-churn benchmark. The five new objects include the replacement Blob. The initial long-prefix diff expands compressed prefixes nibble by nibble; these measurements expose that cost, not justification for changing the canonical leaf threshold. Node/byte counts establish structural sharing for this workload, **not** superiority over Git, Jujutsu, Pijul or a different map layout.

### Stack, storage and real-entry-point checks

Valid deep radix/source inputs overflowed the initial recursive engine on a 2 MiB debug thread stack. Source update/build, collection, metrics, verification and diff now use explicit heap stacks; bounded deep-radix and exact-path-limit fixtures pass in debug and release without shrinking the format's accepted paths. These are regression bounds, not an unlimited resource claim.

The experimental SQLite/raw-object repository adds actual two-connection CAS/dedup, immutable lost-response receipts, exact-closure export/import, corruption/missing-object refusal and process termination before/after SQLite commit. Two storage review findings were reproduced before correction: a failed final repository-directory sync could be bypassed by reopening, and an identical commit between separate receipt/selection reads could make CLI request preparation falsely stale. Every recognized open now completes the installation barrier; preparation uses one SQLite read snapshot. A permission-failure regression and a controlled real-WAL interleaving pin these cases.

The real CLI test writes source larger than the inline threshold, exports it, checks the binary stream with the independently authored Python bundle parser, imports into another store and materializes a fresh directory. Additional tests preserve executable files, literal symlinks and empty directories, refuse existing destinations and symlink parents, and exercise anchored handles during name replacement. The current suite passes 92 Rust tests in debug and release on arm64 macOS 26.7.2; the independent Python suite passes 49 tests. Linux/macOS CI is configured separately; local results do not stand in for an unrun platform.

These tests do **not** qualify disk-full at every write/sync, GC/compaction/reader retirement, supported-filesystem power loss or normal workspace recovery. The [storage owner](storage.md) distinguishes the bounded implemented subset from its release gates.
