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

**Tavi** remains only a provisional CLI/name, **not a selected brand**. The earlier collision screen was too narrow: the [TAVI scientific toolkit](https://github.com/neutrons/TAVI) uses a tavi command; an older [npm project scaffolding CLI](https://npm.io/package/tavi) exists; and a contemporary [team task tracker](https://github.com/devfurkankizmaz/tavi-app) uses Tavi in developer tooling. These are not equivalently material collisions, but the name is less clean than the earlier record suggested. No repository, binary, metadata-path, package or domain rename is authorized by this research.


## 8. Re-evaluation: the core representation is the consequential choice

**Decision retained, with reasons:** authoritative immutable source snapshots plus stable logical ChangeIds, explicit evolution/dependency information and typed conflict objects. The snapshot is a materialized state, not necessarily the most compact way to *express* the change that produced it.

| Model | Strengths | Costs and failure modes | Fit |
| --- | --- | --- | --- |
| Git-like hierarchical Merkle snapshots | Direct random revision reads; structural sharing; efficient equal-subtree diff; binary/unknown-format transparency | Updating a very wide flat tree can rewrite large directory objects; rewriting ancestry changes revision hashes | Strong correctness baseline, but not necessarily the optimal physical tree |
| Canonical changed-file logs (Scion) | Commit admission scales with changed files; ideal for an indexed always-online fleet; independent derived history and blame | Reconstructing arbitrary versions requires derived snapshots/indexes or walking changes; online-first workload assumptions | Useful for changed-path hints and optional draft append flow, not chosen offline source truth |
| Pijul/Darcs-style patch calculus | Identity and commutation of independent edits; conflict resolutions can follow underlying changes | Line-graph/change-graph intricacy; arbitrary binary/move/rename semantics; huge historical queries need maintained materialization; Git bridge | Study separately and compare objectively, do not discard as mere implementation difficulty |
| Jujutsu revisions + operation DAG | Excellent established rewrite/undo/conflict model and backend abstraction | Change evolution alone does not automatically give patch commutation; broad prior art for our UX | Strong baseline and algorithmic reference |
| Structured AST/entity canonical store (Graft-like) | Potentially finer-grained merge, review, provenance and targeted edits | Parser coverage/versions, malformed intermediate code, formatting/comments, macros and binaries; source-byte roundtrip is nontrivial | Optional derived index and merge driver, not universal truth |
| Whole-file or shared-directory CRDT | Attractive convergence for live text | Does not imply semantic correctness, useful ordering, access control, or stable user intent; problematic for binary/large assets | Not general source state |

Why a hybrid snapshot-plus-change model rather than a patch-only one? General projects require exact historical source, including opaque files and incomplete syntax, independent of a replay history. Revision roots supply this invariant directly. Changed-path facts, base/after IDs, stable ChangeIds, evolution edges, and conflict terms enrich behavior **without creating two conflicting authorities**. Do not permit a claimed patch to disagree with its stored resulting root: validate or derive the relation.

**Important distinction:** a stored diff is not automatically a commuting patch. A patch is context-sensitive unless its addressing and algebra define how it composes with other patches. A file-level expected-blob operation can safely detect staleness but does not offer Pijul's reorder-invariant identity. If empirical work identifies much greater adoption value in commuting edits, reconsider before stabilizing the native format.

Additional reference: [Pijul's change graph and conflict theory](https://pijul.org/manual/theory), [Git's merge-ort default and rename handling](https://git-scm.com/docs/merge-strategies), [Jujutsu storage architecture](https://docs.jj-vcs.dev/latest/technical/architecture/).

### Rename and move identity needs its own experiment

Git often infers a rename from before/after similarity; native explicit move operations could preserve identity and avoid inference. But arbitrary editors and scripts manipulate paths, not necessarily AVC commands. A path, file-content hash, filesystem inode, and stable FileId are different facts. Copy creates two descendants; replace-file workflows may change inode without changing logical file identity; hard links and directory moves complicate mapping. Recommendation: record **optional move/copy provenance**, plus exact before/after paths and objects, rather than assuming mandatory persistent FileId is infallible. Benchmark rename + concurrent edit, split/move, case-only renames, duplicate-content copies and external filesystem mutations before freezing this distinction.

## 9. Storage engine: reopen the premature choice

The previous document selected an append-only immutable-object store **plus authoritative SQLite WAL**. That is viable, but not proved best for durability, tiny updates or total write amplification. The key storage decision is **where an acknowledged root transaction becomes authoritative**, not whether the implementation calls itself a KV store.

| Local candidate | Benefits | Liabilities | Proof required |
| --- | --- | --- | --- |
| Single append-only object/commit journal, derived indexes | One sequenced durability stream; object bytes and root-selection records together; rebuildable query state | Custom recovery, segment compaction, root checkpoint pruning and concurrent commit sequencing | Crash/power-loss model, GC/reader safety, whole-workload latency |
| Immutable object segments + SQLite authoritative roots | Short conditional SQL updates; mature indexing; large blobs not copied into SQLite WAL | At least two ordered durability boundaries; staged-object vs GC races; consistent backup of DB + referenced objects | Correct closure/pinning and fsync order; repeated saves and compaction benchmarks |
| SQLite small blobs + metadata, external large blobs | Simple common-case atomicity and recovery; compact small objects; mature backup tooling | WAL/page-copy overhead, checkpoint pressure and concurrency/size constraints; large objects still cross two stores | Source-sized data and high-frequency save performance under FULL sync |

Do **not** conclude from the [SQLite 2017 small-BLOB benchmark](https://www.sqlite.org/fasterthanfs.html) that SQLite is always faster than an append log; it compared particular files, blob sizes and machines. Conversely, don't assume a custom log wins simply because it writes fewer copies. [SQLite WAL mode](https://www.sqlite.org/wal.html) supports concurrent readers with one writer, and its [synchronous settings](https://www.sqlite.org/pragma.html#pragma_synchronous) explicitly distinguish FULL durability from NORMAL on power loss. The [XLL NSDI 2026 paper](https://www.usenix.org/conference/nsdi26/presentation/shawger) reports a 5.5× throughput gain and 73% lower amplification **for TiKV's cross-layer consensus/database logs**, not for source control. It motivates avoiding duplicate payload logs, not adopting its implementation wholesale.

The logical format should remain independent of physical placement, compression, index layout and compaction schedule. A small deterministic reference store can model object identity while the above physical approaches are benchmarked. Avoid a large generic plugin abstraction before at least two alternatives exist.

### Critical protocol races to prove, not assume

- **Prepared object versus GC:** immutable objects written before publication are initially unreachable. If a GC snapshot races the root CAS, it could retire the segment containing a newly referenced object. Pin *staging segments/transactions* durably or coordinate generation barriers before objects can be eligible for collection. A simple final-root CAS alone does not solve it.
- **Segment visibility versus metadata commit:** before referencing a segment, durable contents, object index/footer as required, and directory/manifest attachment must be ordered safely. The acknowledged current root must never depend on an unflushed directory entry.
- **Compaction versus readers:** readers holding old physical locations must continue reading them until leases/epochs expire. Atomic metadata generation changes cannot justify early file deletion.
- **Backup and repair:** an authoritative SQLite DB cannot be discarded like an index. Backup root authority, WAL/checkpoint state, referenced objects and required retention metadata as one consistent recovery point, or use a carefully designed append-root recovery log. Integrity hashes reveal corrupt payloads but do not independently restore them.
- **Idempotency horizon:** deduplicate actor-scoped request identities through explicitly active epochs. Expiry must produce a rejection/unknown state, not quietly execute a delayed old write as new.
- **Physical versus logical write serialization:** a single SQLite writer or append sequencer can serialize short records but **workspace A and B must not CAS one global moving head**. Store independently versioned WorkspaceState and TargetState roots. Multi-resource operations use one conditional transaction. Reconstruct global history views by query, not a mandatory global root update on every local edit.

One workable metadata schema (not a frozen format): immutable Objects; WorkspaceRoot(id, generation, revision, operation, owner); TargetRoot(id, generation, revision, policy_epoch); RecoveryPin(workspace, snapshot, expiry); Proposal/Acceptance; RequestOutcome(actor, epoch, key, digest, result); and GC/ReaderLease generations. Immutable source is content-addressed; these mutable rows are the narrowly defined authorities. The single-journal candidate encodes equivalent transitions rather than requiring SQL tables.

Relevant prior art: [Fossil's immutable artifacts stored in SQLite](https://www.fossil-scm.com/home/doc/trunk/www/fileformat.wiki), [Git's pack and index layouts](https://git-scm.com/docs/gitformat-pack), [RocksDB WAL durability and group commit](https://github.com/facebook/rocksdb/wiki/WAL-Performance). RocksDB and LSM compaction do not inherently simplify immutable CAS plus authoritative root state.

## 10. Persistent trees, hashes, and large content

**Tree shape:** small sorted flat directories are attractive because real source repositories tend to have modest directory widths, and lookup locality is excellent. At extreme width they make incremental updates costly. Compare content-defined paged ordered trees (POS/prolly), deterministic Merkle search trees and conventional immutable B+ trees against a flat-tree reference.

Require:
1. byte-identical content + logical metadata yields identical tree roots regardless of insertion order;
2. a one-file edit rewrites only its ancestor/page path on normal distributions;
3. diff/merge can skip identical subtrees without scanning all files;
4. ordered listing, path-prefix queries and rename/directory move remain efficient;
5. adversarial path prefixes or page boundaries cannot blow up CPU/memory;
6. small projects do not pay a significant object-count or read overhead.

[Dolt Prolly Trees](https://www.dolthub.com/blog/2024-03-03-prolly-trees/) demonstrate independent content-addressed page boundaries. However Dolt's [2025 fast merge work](https://www.dolthub.com/blog/2025-07-16-announcing-fast-merge/) illustrates the difficult case where page boundaries shift despite unchanged keys; a naive node-ID comparison does not make *all* merges proportional to changed entries. Don't guarantee asymptotics before choosing the algorithm and constructing adversarial fixtures.

**File content:** whole small/medium blobs should be the baseline. For frequently edited large binaries, content-defined chunking may reduce transfer/storage at CPU, index and seek cost. [FastCDC](https://www.usenix.org/conference/atc16/technical-sessions/presentation/xia) reports workload-specific speed advantages over Rabin/Gear baselines, not automatic benefit to source files. Compare whole blobs with bounded physical deltas and selective chunks (including already compressed assets). The canonical file identity is the uncompressed source bytes/file manifest; chosen physical packing must not change it.

**Hash choice:** BLAKE3-256 is compelling for SIMD/parallel throughput and provides published vectors; SHA-256 has broader FIPS-regulated ecosystem support. [BLAKE3's implementation/specification](https://github.com/BLAKE3-team/BLAKE3) and [Git's SHA-256 migration design](https://git-scm.com/docs/hash-function-transition) demonstrate why algorithms and conversion maps should be explicitly namespaced. Hash speed alone may be dominated by filesystem scan, compression and durability. Select exact canonical encoding, hash and test vectors together; never identify a Git SHA-1 object by an unrelated native digest.

## 11. Agents, composition, and validation: distinguish actual guarantees

Agent workflows are a **continuum**, not a separate source-control species. Humans, IDEs, refactoring scripts, CI and agents all benefit from exact inputs, isolated workspaces, fast replay, stable change IDs and recoverable operations. A native system must not demand agent identity/provenance just to commit a human edit.

- **Independent physical workspaces** prevent dirty-file interference and let agents run unrelated tests. They are not process, network, build-service or credential sandboxes. [BranchFS](https://arxiv.org/abs/2602.08199) illustrates ultra-cheap isolated filesystem branches; it is an optional physical workspace accelerator, not an alternative history/merge VCS.
- **Shared-directory virtual branches** can help one cooperative human organize changes. [GitButler's parallel branches](https://docs.gitbutler.com/features/branch-management/virtual-branches) support that workflow, but sharing one physical directory doesn't isolate simultaneously executing independent agents.
- **Current Jujutsu** already has [parallel isolated command execution](https://github.com/jj-vcs/jj/blob/main/docs/config.md) and concurrent operation views. Do not claim workspace creation or parallel execution is new. AVC's possible value is combining those with conditional source APIs, bounded recovery and incremental validation under one native transaction model.
- **Read-set observations** are valuable for catching stale reasoning even when changed paths are disjoint (read interface A; edit caller B; another worker changes interface A). [Weft](https://weftgate.com/) implements a capabilities/read-set/evidence model and reports demonstrations. But partial or agent-declared observations are not complete proof of what was read; even complete source reads omit runtime/service environment and intent. Treat them as warnings or explicitly enforced preconditions, never semantic independence certificates.
- **Structural merging** can combine independent declaration/import edits and improve review, as [Graft](https://github.com/odvcencio/graft) and [Mergiraf](https://docs.rs/mergiraf/latest/mergiraf/) demonstrate. Must preserve exact source bytes, invalid intermediate syntax, comments, unknown languages and binary content; parser-versioned drivers are optional. Successful AST merge does not imply compiled or intended software.
- **Verification subjects:** acceptance must reference a candidate, validation recipe, immutable source root and environment/toolchain inputs. Test results/approvals are later immutable evidence *about* a candidate; including evidence inside the candidate digest creates a cycle. SLSA [provenance](https://slsa.dev/spec/v1.1/provenance) offers an established attestation vocabulary, not proof that a test set is complete.
- **Integration:** a target's shared history is ordered even if input production is massively parallel. A merge queue can validate batched composed candidates and avoid unnecessary CI repetition. GitHub's [merge queue](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/configuring-pull-request-merges/managing-a-merge-queue) already verifies combinations against the target. The differentiator is frictionless low-noise *local/draft* workflows and integration contract, not inventing the first merge queue.

A decisive benchmark must include **semantic interference without file conflicts**, change revisions after validation, read-set omissions, test-suite alteration by an agent, and network loss after a target CAS. Do not report "no conflicts" as "correct combined code".

## 12. Local-first sync, portability, security

**Sync:** native immutable-object RPCs can fetch exact tree/blob ranges and resume without always creating packfiles. Merkle traversal naturally supports sparse requests. For network efficiency, compare batched object fetch and compressed bundles against Git protocol v2/partial clone; a network round-trip per tiny object would erase any architecture advantage. Derived ancestry/path/blame indexes can be computed locally or optionally on a server without becoming source truth. Scion's remote cache topology is valuable at massive scale, but ordinary projects should neither require it nor pay its operational cost.

**Privacy:** only explicit restricted draft sync is allowed to upload private recovery. Reachability is an information disclosure boundary: uploading one root also exposes its ancestry/object closure. Content-addresses authenticate bytes, not requesters. Protect against hash-probing and cross-tenant existence oracles; independently authorize object retrieval. Retention must include backups, draft objects and event audit data.

**Transport security matters in addition to signed objects.** [Radicle's September 23, 2026 vulnerability disclosure](https://radicle.dev/2026/09/23/disclosure-of-vulnerability-in-network-protocol) reports that releases to that date transmitted data without encryption/authentication despite signed refs verifying stored content. This is a useful concrete counterexample to assuming content signatures protect private repository traffic.

**Path and checkout security:** Linux permits filename byte sequences that cannot always be materialized on Windows/macOS. Treat path spelling, casefolding, Unicode normalization, invalid components, symlinks, hard links, executable mode, filters and user ownership as separate capabilities. A portable default should refuse alias collisions before checkout and never follow an untrusted symlink outside an authorized workspace. External filesystem rename observation is not always a reliable identity signal.

**Git bridge:** conversion must preserve imported object bytes/IDs *where unchanged* or explicitly track the mappings. Once AVC creates a new native revision, the Git commit's header, parents, signatures, conflict representation and native operation/provenance may not have a lossless counterpart. Define round-trip scope with fixtures for signed commits, tags, submodules, attributes/EOL/filters, LFS, shallow history, renamed files and non-UTF8 names. Reject unsupported cases rather than claiming full compatibility. No live shared-worktree/index writes by two VCS engines initially.

**Recovery:** [Watchman's overflow recrawl](https://github.com/facebook/watchman/blob/main/website/docs/install.md) reinforces that filesystem notifications are hints. Auto-save cannot see unsaved editor buffers or guarantee a cross-file simultaneous snapshot while another process writes. Reports must identify last *durable* capture, exact protected paths and failures.

**Data-safety tests:** [Metis FAST 2024](https://www.usenix.org/conference/fast24/presentation/liu-yifei) and [CrashMonkey/ACE OSDI 2018](https://www.usenix.org/conference/osdi18/presentation/mohan) demonstrate state exploration and crash-consistency bugs even in mature file systems. Combine an independent reference state model with fault injection on object append, sync, root selection, workspace materialization, GC, import, export, and remote commits. Separate process-crash tests from true power-loss guarantees.

## 13. Decision register and evidence priorities

| Question | Current position | Confidence | What could overturn it |
| --- | --- | --- | --- |
| Native core versus Git/jj authority | Native core for maximum long-term product differentiation | Strategic choice, not a measured technical victory | Native workflow gains marginal or contract unachievable |
| Source truth | Immutable Merkle revisions + explicit change evolution | High for general opaque-file/offline requirements | Patch-centric prototype wins complete correctness/performance tasks |
| Semantic history versus recovery | Separate histories and retention; public events intentional | High | Contradictory user workflow evidence |
| Workspace concurrency | Per-workspace logical roots; per-target publication CAS | High | Observed unacceptable coordination overhead or new consistency need |
| Canonical change commutation | Not assumed; treat conditional composition and explicit conflicts rigorously | High | Implementable algebra for all target file types with compelling value |
| Small/wide directory representation | Flat small and deterministic paged large is a strong candidate | Medium | Adversarial tests/lookup/merge results |
| Physical local durability | **Undecided**: single journal vs SQLite+segments vs SQLite small-object store | Open, high priority | Crash correctness and representative measurements |
| Hash | BLAKE3-256 vs SHA-256 with algorithm-tagged IDs | Open | Security/FIPS/performance/interop requirements |
| Large content | Whole blobs baseline; chunks/deltas selectively | Medium | Binary edit/transfer workloads |
| Merge | Exact tree + conservative text + typed conflicts; AST drivers optional | High | Differential merge/round-trip failures |
| Agent read sets | Optional evidence, enforced only when complete by contract | High | Controlled observation/validation system proves wider guarantees |
| Native server | Optional; immutable object RPC and explicit target CAS | High | Local-first requirement changes |
| Git bridge | Required, but declared supported subset | High | User adoption experiments require wider first-release coverage |
| Project name | Tavi **provisional**; collision review reopened | Open | Namespace/usability review and user choice |

### Research-to-implementation proof sequence

1. **Model, not infrastructure:** specify the change/revision/operation/workspace state machine and exact invariants. Create independent model checks for divergence, undo, retention and shared target CAS.
2. **Tree and commit representation:** compare snapshot, snapshot+changed-path hints, and one serious patch-centric prototype on real histories, binary files, wide paths and conflicted rebases. Verify source-byte reconstruction and change identity.
3. **Storage competition:** implement a tiny vertical-slice version of the three local durability options above; fault-inject every persistence barrier and measure real repeated saves, compaction and reads. Select one *before* binding the stable format to it.
4. **Native local product:** workspaces, safe checkout, bounded capture, typed API and deterministic merges—tested against Git worktrees and current Jujutsu. Prove there is a real qualitative improvement, not just a synthetic speed record.
5. **Compatibility and server:** user-visible Git exchange and exact-candidate integration first; optional native sync and cache scale when the core is trustworthy.

No honest claim can be made yet about AVC's performance, production durability, or semantic-conflict reduction. Published upstream numbers remain their authors' workload-specific measurements until reproduced. The first serious improvement is to replace untested architectural assumptions with **clear experiments that can falsify the design**.
