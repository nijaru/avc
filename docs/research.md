# AVC — adversarial architecture decisions and evidence

**Reconciled 2026-11-04.** This is a **decision and falsification record**, not a second source/specification. Authoritative logic: [design](design.md), [canonical format](format.md), [storage](storage.md), [native protocol](sync.md), [Git bridge](git.md), [UX](ux.md), and [roadmap](roadmap.md). The extensive older research corpus, independent prototypes and their full measurements are [preserved in Git history](https://github.com/nijaru/avc/blob/af66a63898f383b596e4828199dc625a80e28a72/docs/research.md); the historical experiment source is [also preserved](https://github.com/nijaru/avc/tree/af66a63898f383b596e4828199dc625a80e28a72/docs/experiments). **Do not reinstate old code/fixtures as normative simply because they existed.**

This review is an independent synthesis of upstream documentation and earlier bounded experiments. It is **not** new native benchmark, interoperability testing, formal proof or fault-injection evidence.

## 1. Strongest alternative: don't build a new VCS at all

Git + Jujutsu + GitButler + an agent harness already provides immutable Git source, stable logical change IDs, operation history, change stacks, flexible UI and broad adoption. [Jujutsu architecture](https://docs.jj-vcs.dev/latest/technical/architecture/) deliberately separates repository data, operation storage, working copy and presentation; [GitButler parallel branches](https://docs.gitbutler.com/features/branch-management/virtual-branches) supports several applied branches in one directory. A Git backend would reduce protocol, storage, migration and ecosystem risk enormously.

**Reason to pursue AVC nevertheless:** the user explicitly wants a genuinely native VCS whose source object model, on-demand fetch, conditional agent edits, structured conflict semantics and offline recovery do not inherit Git's format or client requirements. This is a *product bet*, not a proof of intrinsic superiority. AVC must win at complete source-development workflows and trustworthiness, not novelty or microbenchmarks. **Stop/reassess** if the native implementation cannot deliver meaningful practical gains over Git plus mature clients.

## 2. Scion/Mononoke changesets versus persistent Merkle snapshots

[Anthropic Scion (Oct 13, 2026)](https://www.anthropic.com/research/redesigning-source-control) records canonical first-parent changed-file sets and derives manifests, ancestry/path/blame indexes on servers. [Mononoke Bonsai](https://github.com/facebook/sapling/blob/main/eden/mononoke/docs/2.1-bonsai-data-model.md) applies a similar separation at large scale. These are powerful for extremely high-rate, always-online commits with infrequently materialized source. **Their vendor-measured speedups mix caching, virtual workspaces, distributed metadata, traffic and compute; they are not controlled proof that canonical changesets beat Merkle roots.**

**Selected for AVC:** canonical snapshot roots built **incrementally** via structural sharing; a normal one-file edit need not rebuild or copy the entire repository tree. Direct exact source identity and offline readiness matter on ordinary local and ephemeral workloads. Store changed-path summaries as derived authenticated hints, not a second canonical interpretation.

**Falsification:** if a correct incremental Merkle implementation cannot achieve acceptable edit/clone/query performance while a changeset-first model demonstrably can with comparable offline proof/custody, revisit authority before v1 release.

## 3. Pijul/Darcs causal changes versus exact source

[Pijul theory](https://pijul.org/manual/theory.html) and [Darcs patch algebra](https://darcs.net/Theory/MergersDocumentation) offer a credible stronger model for causal patch composition, selective undo and commuting independent changes. It would be tempting to make such changes the universal repository history for simultaneous coding agents.

**Objection:** arbitrary external filesystem writers only yield observed *result bytes*, not an unambiguous causal edit graph. Renames vs copy/delete, binaries, mode/type changes, and path ownership complicate the canonical algebra. AVC instead defines exact typed source and safe conditional operations; optional recorded moves, hunk ownership, `absorb` and structured editing can exploit known intent without making unknown intent a data-integrity obligation.

**Falsification:** a bounded causal prototype wins whole-task repeated agent merge/rebase cases with arbitrary-file/path correctness and substantially simpler retention than AVC's source-root model.

## 4. Conflict algebra: deliberately change the November 3 preference

The older AVC draft stored **nested opaque conflict alternatives** and used an all-optimal-alignment text algorithm to minimize false-clean outcomes. This preserved uncertainty but generated extra conflicts and a depth budget in repeated history changes. The earlier recorded 64-triple historical sample showed **54 clean/10 conflicts** with its refined conservative driver versus **58 clean/6 conflicts** for Git and Jujutsu. Inputs came from only 14 sampled Git merges and are **not a semantic correctness oracle**. [Original cohort and limits](https://github.com/nijaru/avc/blob/af66a63898f383b596e4828199dc625a80e28a72/docs/research.md#21-sparse-ambiguity-without-a-dense-fallback--october-29-2026).

[Jujutsu's technical conflict model](https://docs.jj-vcs.dev/latest/technical/conflicts/) represents tree expressions of the form `A+(C-B)`, flattens nested expressions and cancels equal opposite terms. This prevents an unresolved conflict from growing into nested conflicts during repeated rebases. Its **same-change rule is explicitly lossy** under later rebasing. AVC adopts the valuable algebraic normalization but **not** unconditional lossy resolution or a requirement for canonical whole-tree expressions.

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

The code reset is intentional, authorized and not a claim that no earlier research was useful. The next local session must implement from **current contracts**, with fresh golden vectors and real failure-path checks.

## 10. How to overturn a decision

An overturn requires one specific violated invariant, an essential workflow that cannot be supported, or a controlled material end-to-end advantage. State the exact input/conditions, competing contract, data-loss/security implications and migration impact; then update **one authoritative owner** and its dependent implementation. Avoid indefinite algorithm menus, unversioned format changes after release, and marketing claims of globally optimal performance. The [roadmap](roadmap.md) now begins implementation, not further architecture exploration.
