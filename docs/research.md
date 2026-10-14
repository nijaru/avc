# Research: ground-up source control for human and agent work

**Reviewed:** 2026-10-14  
**Decision:** a native, local-first source-control architecture centered on immutable **source state**, independently versioned **authored work**, and context-specific **applications/integration**.  
**Evidence status:** architectural reasoning and upstream primary sources; **no native AVC performance or durability benchmark has been run**.

The maintained technical contract is [design.md](design.md); implementation falsification gates are in [roadmap.md](roadmap.md). This document replaces earlier cumulative research notes rather than treating each incumbent's strongest feature as an AVC requirement. Previous versions remain in Git history.

## 1. Start from requirements, not incumbent structures

Existing commits, branches, patches, operation logs, and trees answer overlapping questions. A new VCS is worth building only if it can improve *complete tasks*, not just move a familiar representation into a novel object store.

General projects require exact opaque source bytes, fast ordinary editing, recoverability, offline operation, history inspection, merge/conflict semantics, binary/rename handling, review and exchange with today's ecosystem. Agent-heavy workflows add more independent attempts, short edit/test loops, stale observations, abandoned experiments, frequent source queries, and greater pressure on integration and validation. None of this requires a model name, prompt transcript, issue tracker, or agent scheduler inside source truth.

**Constraint, not a forecast:** many workers should not rewrite or lock one global mutable head for unrelated local work. A shared accepted target still must resolve a linear sequence of promotions, even when hundreds of workers prepare in parallel.

**Strategic choice:** don't embed Git or Jujutsu as authoritative native storage/history. This choice offers architectural freedom but **is not evidence of superior safety or performance**; it increases our independent implementation and verification obligations. A Jujutsu-based extension is the strongest fallback if the native design fails to create major user value.

## 2. Compare fundamentally different source/history models

| Core representation | What it does best | Difficulties inherent to the model | Judgment |
| --- | --- | --- | --- |
| Snapshot/commit DAG (Git, Mercurial) | Any revision reconstructs directly via persistent trees; opaque files; content sharing; simple historical identity | Rebase rewrites ancestry-linked revision identity; intent/independence not fully represented | Excellent state baseline, incomplete authored-work semantics |
| Snapshot with stable work/revision separation (Jujutsu) | Excellent editable history, first-class conflicts and undo; proven user model; pluggable storage | Work revision application still coupled to ancestry-based rebases; operation views can become a broad coordination concern | Very strong alternative; most conventional AVC benefits already exist here |
| Canonical incremental changed-file log (Scion) | Change admission independent of whole repository size; server-side derived indexes/cache; very cheap concurrent change creation | Arbitrary state/history queries need materialized indexes; server-first infrastructure; offline reconstruction has different costs | Excellent scale model, not universal local default |
| Patch calculus / change graph (Darcs, Pijul) | Changes can preserve identity across contexts; declared commutation and conflict resolution may be reusable | Universal path/file/rename/binary semantics; ambiguous application; full-history inspection and interop are nontrivial; algebraic bugs consequential | **Serious alternative that must get a real prototype** |
| Operation/event sourcing only | Cheap incremental append, strong audit and branching of events | Recovering arbitrary exact source without snapshots may require expensive replay; event and filesystem authorities easily diverge | Good for recovery/provenance, not sole universal source representation |
| Text CRDT or AST/entity graph as source truth | Local concurrent text edits; possibly structured merges and semantic queries | Does not establish program correctness; languages/macros/invalid syntax/binaries, formatting and parser versions cannot be ignored | Optional derived facilities, not general canonical truth |
| **Source State + WorkVersion + Application** | Exact state and stable authored identity; apply work in multiple contexts without modifying authorship; explicit integration subject | WorkVersion→new context still needs merge/patch transformation and correct conflict preservation; complexity must be hidden from normal UX | **Leading native design** |

The leading design is not equivalent to storing one stable ChangeId on a Git commit. An immutable **authored WorkVersion** records its own base/result. Applying it to another context creates an **Application record** and resulting State, without replacing the authored object. That separation permits many agents to submit work against old bases and compose a group onto one newer target. It does not abolish conflicts or stale semantic assumptions.

The **State** is the byte-authoritative output. If a delta and materialized state are both stored, the delta must be verified against the base/result; they cannot be two sources of truth with independent update paths.

### 2.1 Why snapshot state still leads

Source files can be malformed, binary, generated, symlinked, renamed through generic filesystem calls, or changed outside any VCS API. Exact source state is a necessary product output. A persistent Merkle root allows direct authentication and retrieval of that state, subtree sharing and equality checks. This is an inference from requirements, not a claim that Git's tree encoding is optimal.

The interesting research is whether a **canonical change graph with derived materialized States** could be even better. Pijul stores changes and maintains a conflict-tolerant materialized pristine; Scion writes file changes and recursively derives query indexes. Both show alternatives to forcing every logical change into a Git-style snapshot commit. A patch-graph candidate must be compared on arbitrary history reconstruction, materialization/replay cost, byte preservation, merge/conflict algebra, binary edits and reapplication. Do not reject it simply because it is harder to implement.

### 2.2 The patch algebra bar

Darcs describes patches as *identity + context + effect* and defines commutation transformations. Pijul uses a change graph with dependency/conflict semantics and claims independent changes commute. This is stronger than diff3 or a stable display ID. Real claims require laws/tests for identity, contextual transformation, inverse operations and conflicts, including ambiguity cases. A patch that applies successfully to a new base does not prove the change still has the intended behavior.

The decision to keep snapshots leading should be **reversed** if a prototype establishes a safer or more expressive general model with competitive arbitrary-version reads and binary/rename behavior.

Sources: [Darcs mergers and commutation theory](https://darcs.net/Theory/MergersDocumentation), [Darcs conflictors](https://darcs.net/Theory/Conflictors), [Pijul rationale](https://pijul.org/manual/why_pijul.html), [Pijul channels](https://pijul.org/manual/workflows/channels), [Pijul conflicts](https://pijul.org/manual/conflicts.html).

## 3. What current systems actually demonstrate

### Git

[Git objects](https://git-scm.com/book/en/v2/Git-Internals-Git-Objects), [pack format](https://git-scm.com/docs/gitformat-pack), [worktree](https://git-scm.com/docs/git-worktree), and [partial clone](https://git-scm.com/docs/partial-clone) document immutable shared source objects, physical deltas, separate working directories and sparse object promises. Commits do **not** copy every file on every revision.

Implication: the novel VCS must not claim content addressing, snapshots, or isolated checkout are inventions. The measurable opportunity is reduced history churn, faster group composition, simpler recovery, conditional machine operations and cross-layer consistency.

### Jujutsu

[Architecture](https://docs.jj-vcs.dev/latest/technical/architecture/), [operation log](https://docs.jj-vcs.dev/latest/operation-log/), [concurrency](https://docs.jj-vcs.dev/latest/technical/concurrency/), and [conflicts](https://docs.jj-vcs.dev/latest/conflicts/) document library/backend separation, editable stable changes, conflict-bearing revisions, and operation/view history with concurrent divergences. [Working copy documentation](https://docs.jj-vcs.dev/latest/working-copy/) explains its auto-snapshot/replacement behavior and stale working-copy recovery.

JJ is a meaningful comparator even if AVC uses a new store: it already handles much of the human experience we want. Its operation-view DAG can reconcile concurrent mutations; the current docs also acknowledge backend/concurrency caveats for certain Git-backed setups. Our model's possible improvement is not just an operation log. It is avoiding **authored WorkVersion rewrites solely due to application to another base**, and scoping coordination to the resources truly changed.

### Anthropic Scion

[Anthropic, Oct 13, 2026](https://www.anthropic.com/engineering/redesigning-source-control) documents a **canonical changed-file list per changeset**, with derived ancestry, file history, blame and search indexes; immutable-object query services, on-demand file materialization and tiered caches. Scion separates cheap changeset creation from branch CAS. Reported workload: more than 30,000 merged changes/day and around 25 million CI checkouts/day; reported infrastructure gains include roughly 200× lower CPU/memory footprint versus the former mirror fleet and roughly 100× faster fleet checkouts. **Those are Anthropic's comparisons, not independent native-AVC speed forecasts.**

Implication: make checkout-free operations standard, do not move public refs on private edits, let immutable objects be fetched by ID and derive indexes recursively. But a 25 GB monorepo worked on by always-online agents is not the default for general software projects. Local offline correctness remains valuable. Their post-merge/pre-deploy testing strategy is one explicit organizational risk choice, not universal policy.

### Meta Sapling / EdenFS

[Sapling's scale overview](https://sapling-scm.com/docs/scale/overview/) documents separate evolution of on-demand blob/tree/commit fetching, incremental working-copy status, sparse checkout, virtual materialization, commit-graph indexes and automatic backup. [EdenFS](https://github.com/facebook/sapling/blob/main/eden/fs/docs/Overview.md) provides optional lazy filesystem access for huge repositories.

Implication: different scale dimensions need distinct solutions. Sharing immutable objects, avoiding redundant checkouts and making materialization optional belong at the API boundary. A virtual filesystem must not become mandatory just because some agent fleets benefit from it.

### Mercurial / Fossil / Gerrit

[Mercurial evolution](https://www.mercurial-scm.org/help/topics/evolution.html) and [phases](https://www.mercurial-scm.org/help/topics/phases.html) separate obsolete/successor history and public/draft visibility. [Gerrit Change-Ids](https://gerrit-review.googlesource.com/Documentation/user-changeid.html) distinguish change identity from patch sets. [Fossil's file format](https://www.fossil-scm.org/home/doc/trunk/www/fileformat.wiki) distinguishes logical immutable artifacts from storage delta choices.

Implication: stable work identities, evolution, public/private semantics and physical delta independence all have extensive prior art. AVC's value must be the coherent **authored versus application** distinction and end-to-end workflow, not individually reinventing familiar concepts.

### GitButler / Trellis / Weft

[GitButler parallel branches](https://docs.gitbutler.com/features/branch-management/virtual-branches) allow one human to organize multiple branches in a shared working directory. Its [parallel-agent documentation](https://docs.gitbutler.com/ai-agents/parallel-agents) distinguishes shared runtime benefits from worktree isolation. A shared directory remains a poor default for independently executing workers.

Trellis's accepted [ADR 0038](https://github.com/trentbrew/trellis/blob/main/docs/adr/0038-git-authoritative-file-tier.md) recounts **two project-reported incidents** where an op-log materializer reverted source changes; the maintainers subsequently chose Git as the sole byte authority and relegated the op log to annotations. [ADR 0054](https://github.com/trentbrew/trellis/blob/main/docs/adr/0054-lane-scoped-file-ownership.md) reports ownership failures where multiple sessions shared one agent identity. These are developer-reported incidents, not independent field studies.

Implication: do not create independently mutable byte authorities. Even **one native object authority** still has to treat the editor-owned filesystem as an external mutable state and protect against stale checkout/restoration. Agent identity is not equivalent to workspace/session identity. [Weft](https://weftgate.com/) provides useful prior art in capabilities, input observation and evidence-gated operations, but observations cannot prove semantic correctness.

### Structural merge / incremental computation

[Mergiraf](https://mergiraf.org/architecture.html) and [Graft](https://github.com/odvcencio/graft) explore syntax-aware or structural source merging. They may improve conflict detection and ergonomics when grammar/tooling supports the file. They do not provide a universal byte-preserving merge for arbitrary languages, invalid files, macros and binaries. An LLM-suggested resolution must remain a proposed authored change.

[Salsa's incremental query model](https://docs.rs/salsa/latest/salsa/) tracks dependencies and recomputes only invalidated derived values. Scion's derived-data strategy is a closely related *architectural pattern* for efficient repository queries; AVC can implement versioned derived indexes without adopting a compiler's specific query runtime.

### Persistent trees and physical logs

[Dolt Prolly Trees and fast merge](https://www.dolthub.com/blog/2025-07-16-announcing-fast-merge/) show both content-defined persistent ordered map benefits and the hard case of shifted page boundaries during merge. Do not assume Merkle equality makes **all** merges proportional to the number of changed files.

[SQLite WAL](https://www.sqlite.org/wal.html) and [synchronous PRAGMA](https://www.sqlite.org/pragma.html#pragma_synchronous) distinguish transactional metadata from guaranteed power-loss durability; WAL with synchronous=NORMAL may lose acknowledged recent transactions on power failure. [SQLite's atomic commit discussion](https://www.sqlite.org/atomiccommit.html) describes storage ordering assumptions. A custom append log can reduce redundant durability layers, but owns log integrity, compaction and repair. There is no proven winner for AVC yet.

## 4. Why one coherent model beats an inventory of features

The leading State/WorkVersion/Application model answers five awkward scenarios without imposing a public branch or a rewrite of authored work:

1. **Independent agents:** A and B author against S0; compose A+B once onto S1. A/B authored versions stay unchanged; the Application records the exact combined result.
2. **Conflicting edits:** A and B change overlapping bytes or file types; composition retains both alternatives as typed conflict objects. A user can preserve both attempts without prematurely choosing a winner.
3. **Stale reasoning without overlapping paths:** B edits a caller while A changes an interface. Mechanical composition can succeed, but optional observed input hashes warn and exact-candidate validation can fail. No semantic independence is claimed.
4. **Fast local experimentation:** private recovery captures preserve intermediate bytes without creating permanent WorkVersions, triggering CI or rewriting descendants.
5. **Human workflow:** a single developer sees one active change and one working directory; Applications and WorkVersion lineage are available for power users and tooling, not mandatory command vocabulary.

Compared with normal revisions/rebases, this *might* reduce unnecessary rewritten history and repeated revalidation of authored content. Yet composing an authored delta onto a changed base can still generate conflicts and **must** validate a new integrated result. There is no general algorithmic guarantee of constant-time reapplication or semantic safety.

## 5. Explicitly rejected assumptions

- **“A native storage format is automatically better than Git.”** False. Need full lifecycle measurements.
- **“Git commits contain full physical file copies.”** False. Merkle object sharing/packing already exists.
- **“A stable ChangeId yields commutative changes.”** False. Identity, context and effect need actual transformation laws.
- **“No overlapping file paths means changes are independent.”** False. APIs, config and runtime contracts cross files.
- **“Every agent should have a branch or public ref.”** Not required. Local work and publication are separate.
- **“Every repository transaction needs one moving repo-wide head.”** Not required. Resource-scoped roots can share a physical log with no spurious *logical* contention.
- **“A successful metadata commit means checkout finished safely.”** False. Mutable filesystem changes need an independent no-clobber materialization protocol.
- **“Always-online agents mean an offline mode is obsolete.”** Not true of general projects, mobile/offline work, air-gapped systems and independent developer trust.
- **“Correctness risks of native and layered systems are equal.”** Unsupported. Different failure surfaces; both require evidence. Native storage adds independent trusted code, and layer boundaries can introduce dual-authority hazards.
- **“A merge queue or semantic agent metadata proves correct code.”** False. Candidate-specific tests/approvals offer bounded evidence, not a program theorem.

## 6. How the design could be falsified

| Hypothesis | Evidence that would change direction |
| --- | --- |
| Authored/application separation creates real value | Workflows are no better than Jujutsu amendments + Git worktrees once full composition/validation costs are included |
| Snapshot state should be authoritative | A true patch graph demonstrates safer conflict and rename behavior, equal/stronger arbitrary-version reads, and comparable bytes/runtime |
| Per-workspace roots avoid contention | Multi-workspace transactions and actual storage writer become excessive bottlenecks despite no logical conflict |
| Native physical object engine should be novel | SQLite or a proven embedded KV engine is faster/safer through recovery, GC and steady-state edit loops |
| Checkout-free operations matter | Source query workloads are dominated by external tools that require full materialization, with little gain from immutable APIs |
| Native sync reduces agent-era costs | Remote review/CI/event systems still run on every attempted update or existing Git partial-clone/bundled transfer matches end-to-end costs |
| Migration is acceptable | Real projects require too much unsupported Git behavior or lossy history conversion |

**Benchmark intent:** tuned current Git/Jujutsu, a small native baseline, patch-algebra prototype, and at least one complete agent integration workflow. Record full latency distribution, CPU, memory, disk reads/writes, storage retention, sync traffic, conflict/staleness outcomes, test work and adoption friction. Use real repositories plus synthetic stress cases. Never convert Anthropic's fleet-specific published multiples into AVC promises.

## 7. Naming remains separate

The name **Tavi** is provisional and not a design constraint. Existing unrelated uses include a [TAVI visualization toolkit](https://github.com/neutrons/TAVI). The name needs a dedicated developer-tool/CLI/package/namespace screen and user selection before any repository, executable or metadata-path rename. The architecture should not be bent to make an acronym.

## 8. Bottom line

The native design is **not** “Git snapshots + Jujutsu operation log + Scion cache + agent metadata.” It is an explicit separation of immutable **source state**, immutable **authored work**, immutable **application result**, bounded **recovery**, and conditional **accepted publication**. Each has an independently useful identity and a distinct correctness boundary.

The most consequential unproven alternative is **patch-algebra-as-authority**, not the specific tree or hash algorithm. The highest physical-risk decision is how complete objects, short root transactions, garbage collection and working copies remain durable without accidental data loss. Keep both open until targeted prototypes and fault tests discriminate them.
