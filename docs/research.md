# AVC — adversarial design review and evidence

**November 3, 2026.** This is the maintained *rationale and falsification record*, not a second specification. The active commitments live in [design.md](design.md), the user workflow in [ux.md](ux.md), and qualification steps in [roadmap.md](roadmap.md). The previous **187 KB** chronological research file (sections 1–31), dated experiments, rejected algorithms, raw results and reproduction commands remain intact in [Git history at the pre-review commit](https://github.com/nijaru/avc/blob/94dd9921ee8a93b89551cbecf53bb14f16f08b01/docs/research.md). Do not erase the executable [reference experiments](experiments/) merely because their old codec is superseded; mark their limitations and replace only when a better validated check takes ownership.

**Interpretation:** Observed upstream architectures and limited old AVC Python/reference results are evidence, not AVC native production performance. User explicitly authorizes redesign without allegiance to previously implemented code or a prior assistant's choice. The objective is a practical, high-confidence implementation path, not a proof of universal global optimality.

## 1. Steelman the strongest competing architectures

| Alternative | Best argument *against* current AVC | Reason AVC selects differently | Falsification trigger |
| --- | --- | --- | --- |
| Git + Jujutsu/GitButler, or a Pando-style Git backend | Already working adoption, mature Git history/storage, amend/stack UX and excellent hosted scaling. Removes years of codec, Git and reliability engineering. | User intends a **native** system with first-class checkout-free source operations, native object IDs/protocol, exact history/conflict/recovery semantics and agent-scale selective fetching. Git remains an early adapter. | If complete real Git developer/agent workflows are as safe and fast and AVC's native model provides no meaningful benefit, stop investing in redundant native storage. |
| Scion / Mononoke **canonical changesets** | Incremental first-parent file-change records can accept enormous commit rates without eagerly computing all tree/index data. Derived manifests can evolve independently of canonical IDs. | For arbitrary local/offline editing, exact source roots are frequently needed immediately for checkout, review, diff, test and exchange. Incremental persistent tree path-copy creates few new nodes and provides direct source identity. | Equal-durability, equal-workload comparison shows authored full source roots cost materially more or impair scale, without useful read/recovery gains. |
| Pijul / Darcs **causal patch algebra** | Independent changes may commute; selective composition, partial repository work and conflict retention are native, not simulated via repeated rebases. This is the strongest multi-agent counterproposal. | External editors and arbitrary binary/path/tree modifications do not provide unambiguous causal intent. A universal causal representation carries high correctness and operational surface area for source AVC must preserve exactly. | A bounded prototype supports arbitrary binary/rename/delete/mode changes and repeatedly wins real edit/merge workflows without loss of source custody or excessive conflict complexity. |
| Jujutsu **tree merge expressions** | Flattened multi-sided expressions can simplify repeated rebase/revert and avoid nesting explosion; unresolved conflict is first-class. | AVC first needs a provably lossless, bounded *source value* contract that works with directories and binaries. A universal signed-term algebra is not safe by analogy. | If the simple typed conflict expression grows without bound, erases valid alternatives, or makes common rebases unusable, select verified flattened or tree-level expressions before format freeze. |
| GitButler **one composed workspace** | Multiple stacks, hunk attribution and human review in one directory are substantially nicer than checkout/worktree proliferation. | This is desirable as a *view*, but does not authorize same-file concurrent writers or replace an exact revision/merge model. Separate writable agent workspaces or conditional source updates must remain valid. | Users/agents consistently benefit from shared composed editing without confusing attribution/overwrite, so broaden it earlier. |
| Delta/CRDT-native source | Causal collaborative edits can improve repeated parallel text updates and preserve fine-grained attribution. | General VCS has binary, paths, external writers, offline custody and authored reviews; CRDT convergence doesn't prove semantic or filesystem correctness. | End-to-end workflows demonstrate a large advantage that cannot be implemented as an optional editor/runtime integration. |

**References:** [OpenAI Pando (Nov 2, 2026)](https://openai.com/index/scaling-git-at-openai/); [Anthropic Scion (Oct 13, 2026)](https://www.anthropic.com/research/redesigning-source-control); [Meta Mononoke architecture](https://github.com/facebook/sapling/blob/main/eden/mononoke/docs/1.3-architecture-overview.md); [Jujutsu architecture](https://www.jj-vcs.dev/latest/technical/architecture/); [Jujutsu conflicts](https://github.com/jj-vcs/jj/blob/main/docs/technical/conflicts.md); [Pijul theory and conflicts](https://pijul.org/manual/why_pijul.html); [GitButler parallel branches](https://docs.gitbutler.com/overview); [Sapling absorb](https://sapling-scm.com/docs/commands/absorb/).

### The "snapshots versus changesets" false dichotomy

Git stores exact snapshot roots; Scion/Mononoke canonically store changed-file records and derive trees. These are **different authority choices**, but they do not imply one requires copying a complete tree on every edit. AVC's preferred design stores exact immutable source roots, incrementally path-copies changed Merkle nodes, and may preserve a small authenticated changed-path summary for efficient writes, history and transport. The changed-path data is not a competing canonical interpreter.

A changeset-first design **really would win** if AVC's primary market became millions of hosted append-only changes rarely read, where generating even new path-copied tree roots dominates write cost, and offline authenticated source roots are no longer important. Neither upstream announcement isolates the cost of canonical changesets from caching, working copy, storage, API and hardware differences. Do not cite their published speedups as proof of native-format superiority.

## 2. Old AVC experimental evidence and its limits

### Canonical directory maps

The old map access experiment on a **single pinned version of git/git**, with 4,582 regular-file/symlink bindings and 64 present/missing path samples, compared surrogate layouts with equivalent logical data. Approximate reported outcomes:

| Layout | Unique maps | Total map bytes | Median present-path lookup bytes | New map bytes for one edit |
| --- | ---: | ---: | ---: | ---: |
| Flat directory | 222 | 260,679 | 94,525 | 98,831 |
| Packed radix (old 4 KiB leaves) | 479 | 260,437 | 4,168 | 6,795 |
| One-key radix | 7,098 | 484,998 | 1,951 | 1,639 |

Lower byte reads did **not** always mean fewer object requests: the measured median present lookup used about 3, 7 and 12 map objects, respectively. These were Python/surrogate map payloads and SHA-256 surrogate references, **not native AVC BLAKE3/SQLite/packfile benchmarks**, and exclude physical compression, OS caching and real syscall overhead. The move experiment still visited thousands of descendants to validate path bounds without a proven cached summary. [Original evidence, exact inputs and reproduction](https://github.com/nijaru/avc/blob/94dd9921ee8a93b89551cbecf53bb14f16f08b01/docs/research.md#20-public-history-and-map-access-costs--october-29-2026).

**Implication:** path-compressed radix is a rational implementation baseline, not a fully optimized *canonical leaf size*. Replace the old full-map builder with persistent edits, compare exactly one strong chunked ordered-map candidate, and select byte format from native end-to-end costs. This is the bounded pre-freeze test, not permanent design indecision.

### Text merge and conflict cost

On a limited cohort of **64 file triples from only 14 historical merges**, the original exact-confluence driver merged 40 clean/24 conflicted, and its revised sparse algorithm merged **54 clean/10 conflicted**; Git and Jujutsu each merged **58 clean/6 conflicted** under the recorded comparison. All 54 AVC clean outputs matched recorded historical bytes, but that does **not** prove semantic correctness, and the cohort/selection is biased. The old sparse method also significantly reduced a single reference allocation peak; it did not establish native memory/CPU advantages. [Original study and limits](https://github.com/nijaru/avc/blob/94dd9921ee8a93b89551cbecf53bb14f16f08b01/docs/research.md#21-sparse-ambiguity-without-a-dense-fallback--october-29-2026).

**Implication:** do not preserve the candidate driver's unusual ambiguity rules solely because old fixtures pass. Compare a practical versioned three-way merge, exact typed alternatives and flattened-expression handling on full repeated-edit tasks. The key metric is **unnoticed incorrect clean merges plus extra manual resolution and computational cost**, not clean count alone.

### Local custody

The old SQLite custody and symbolic work-lifecycle models tested narrow ordering/retention properties, not power-loss durability, full filesystem races or physical export/restore. The current Rust verifier reads owning closure and reconstructs directory maps; as-is, that is a costly admission path for large repositories. Its source code and reference tests are **not** evidence of incremental verification or secure native repository operations. [Old implementation baseline](https://github.com/nijaru/avc/blob/94dd9921ee8a93b89551cbecf53bb14f16f08b01/docs/roadmap.md).

**Implication:** implement one real root transaction, file capture, materialization and backup slice. Use SQLite-only as the **equal-durability control** when choosing whether immutable payload segments justify fsync/retirement/backup complexity. [SQLite WAL and synchronous](https://www.sqlite.org/pragma.html#pragma_synchronous) requires FULL for durability across power loss in WAL mode; NORMAL's atomicity does not equal full durability.

## 3. The hidden Git adoption obstacle

The November 3 draft's "clone a large Git repository sparsely, then immediately write native AVC history" promise was too strong.

A Git partial clone can know Git tree entries and Git object SHA IDs without possessing the referenced blob bytes. AVC's native BLAKE3 BlobId is a function of **the blob bytes** and native type domain. There is no SHA-to-BLAKE3 conversion for unknown content. A canonical AVC directory map referring only to these absent BLAKE3 IDs **cannot be built** from a sparse foreign Git tree.

Potential alternatives:

1. **Eager convert current tip:** fetch full selected Git source, generate true native root; historical Git ancestry remains a truthful frontier. Good first release for normal repos; expensive for giant tips.
2. **Foreign virtual view:** treat Git refs/tree/blob IDs as *adapter-only authority* until hydrated, serve Git-relative queries/edits with Git semantics, never claim a native source root prematurely. Enables large Git source exploration but honest about native authoring limitations.
3. **Canonical multi-hash/foreign entries or external-base deltas:** support Git IDs directly inside native source references, allowing lazy native-like editing. This sacrifices "identical source bytes ⇒ identical native source identity" across imported and native representations (unless expensive reconciliation rewrites roots), increases trust/security and backup requirements, and embeds foreign semantics into the core.

**Decision:** implement (1) and scoped (2), not (3), until the latter demonstrates enough complete-task value to justify permanently expanding the native object contract. Separate Git mapping from native history. Do not manufacture a fake root commit when parents are shallow; a persistent unresolved ancestry frontier is part of import metadata. Native remote sparse clients are fundamentally different: the remote already has an authenticated native full root and can verify partial edits.

[Git partial clone](https://git-scm.com/docs/partial-clone) explicitly distinguishes missing promised objects from corruption and notes inefficiency of per-object fetches. [GitHub's data-driven clone study](https://github.blog/open-source/git/git-clone-a-data-driven-study-on-cloning-behaviors/) showed some shallow clones cheap initially but shallow fetches much more expensive in their tested cases; results are not a universal prohibition on shallow history. Measure the repeated user journey, not clone bytes alone.

## 4. Storage and systems performance

[OpenAI Pando](https://openai.com/index/scaling-git-at-openai/) provides the right lesson on **physical** scaling: independent per-ref transactions, immutable reusable transfer objects, cacheability, background LSM-like pack-cover compaction, controlled writer pack dependencies and parallel packfile-URI fetching. Its approximately 5x push gain and ~1 PB/day are against OpenAI's earlier Git backend, not AVC. It also illustrates why caching and coordination can dominate the choice of hash/compressor.

[Scion](https://www.anthropic.com/research/redesigning-source-control) uses an online-only source service, first-parent canonical changesets and derived indexes, node/region caches, sparse/virtualized clients, and no mandatory Git checkout. Anthropic's stated 100x CI checkout improvement and ~200x smaller infrastructure footprint measure its unusual deployment relative to its old mirrors; not a direct contest against native snapshots.

[Mononoke](https://github.com/facebook/sapling/blob/main/eden/mononoke/docs/1.3-architecture-overview.md) offers particularly useful abstractions: small canonical write path and independently derived history/blame/manifests/source indexes, stateless protocol frontends and persistent immutable object stores. AVC can use those separations while remaining local-first.

[BLAKE3](https://github.com/BLAKE3-team/BLAKE3-specs) is a sensible typed content hash. [Zstandard](https://facebook.github.io/zstd/) is a sensible physical compression candidate; dictionaries and chunking may help tiny or related files, but Zstandard-per-object may lose to Git's cross-object delta packing in storage. An end-to-end comparison must include reused pack transfer, random decompression, memory, CRC/ID checks, network round trips, CPU, SSD write amplification and disk. Compression/packing/chunking should not change canonical source IDs.

SQLite metadata + immutable segments is chosen because large objects can stream independently of database WAL churn and may be compacted without rewriting source IDs. **Strongest objection:** split durability across a DB and filesystem creates more crash/GC/backup failure modes than a single SQLite database. That is why a physical backend comparison with equal failure guarantees is a release gate; take SQLite-only if hybrid's advantage is too small.

## 5. Human and agent user experience

[Jujutsu](https://www.jj-vcs.dev/latest/technical/architecture/) demonstrates why mutable repository views and exact immutable authored history should be separate. Its operation backend and view do not make every selection an authored commit; retaining complete operation history requires retention policy. Its Git backend stores ChangeIds and predecessor metadata outside Git objects, illustrating both the value of stable change identity and the cost of foreign-model translation.

[Sapling absorb](https://sapling-scm.com/docs/commands/absorb/) assigns edits to unambiguous earlier stack commits, previews by default and leaves ambiguous changes alone. This is an excellent **optional change-stack UX** but does not justify trusting inferred intent as canonical source truth.

[GitButler](https://docs.gitbutler.com/overview) shows parallel and stacked changes in a shared working directory. It is persuasive for human workflow. However, branch ownership cannot prevent two processes from racing on a file or incompatible generated build state. AVC should offer derived composed views, independent directories and checkout-free conditional APIs without making any one physical arrangement mandatory.

[Pijul](https://pijul.org/manual/conflicts.html) shows that conflict and graph-level change composition can be modeled natively; a resolved conflict is itself a change. The counterargument to AVC is serious for independent agent edits, but there is no evidence yet that a universal causal patch store solves arbitrary source materialization, binary/path edits, foreign adoption and operational safety at lower whole-task cost.

## 6. Explicitly rejected overengineering

- **No canonical global provenance ledger or replayable operation history inside every source object's hash.** Detailed attestations live outside source identity; construction hints are not a second authority or a reason to retain private discarded source.
- **No mandatory daemon, FUSE, hosted registry, agent runtime, code-review server or CRDT editor.** Their advantages are optional integrations.
- **No automatic directory per agent or forced shared workspace.** Select by concrete write/environment interference.
- **No infinite benchmarking matrix.** A real incremental radix implementation + one alternative, conflict expression qualification, and equal-durability physical storage are the pre-freeze decisions. All others can be implemented against selected invariants.
- **No Git packed objects inside canonical AVC source just for adoption.** Preserve foreign Git mapping, not false complete-native conversion.
- **No "100% optimal" performance claims without workload measurements.** A design optimized for low-latency offline editing can differ from one for 30,000 always-online commits/day.

## 7. Concrete reconsideration conditions

Reopen **canonical authority** if incremental persistent snapshot construction demonstrably blocks exact-source operations at required throughput and a changed-file authority preserves offline correctness better. Reopen **conflict algebra** if typed alternatives lose inputs, grow without bound in ordinary rebase workloads, or cause excessive user conflicts. Reopen **hybrid physical storage** if crash, GC, export or operational cost exceed a simpler SQLite-only design at equal workload. Reopen **Git foreign partial mode** if user experience requires immediate native writable sparse import and a fully modeled mixed-ID solution can preserve canonical equality/security.

Do not make format or feature changes for novelty, incumbent fashion, or to preserve old prototype tests. Each overturn needs an exact counterexample, benchmark with controls, or essential workflow failure. The current [architecture](design.md) is sufficiently settled to implement now.
