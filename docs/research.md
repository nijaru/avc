# VCS research and architecture decisions

**Researched:** 2026-10-13  
**Decision:** Local-first, change-oriented VCS; Git/Jujutsu reuse first; explicit concurrent-work and integration contracts

[design.md](design.md) owns the architecture and [roadmap.md](roadmap.md) owns implementation order. This record retains the external evidence and rejected alternatives that explain those choices. It replaces the old research prompt rather than keeping another unexecuted plan.

## 1. Conclusions

The strongest general-purpose design is not Scion reduced in size. Preserve offline work, ordinary directories, understandable history, and existing hosting. Borrow scalable object/query boundaries without making a server necessary for routine development.

Agent workflows increase the value of independent workspaces, cheap abandoned alternatives, explicit machine preconditions, bounded recovery, and reproducible integration. They do not make source-text convergence equivalent to software correctness. They also do not establish that every project needs a new canonical object format.

The recommended implementation reuses the parts with the strongest fit: Jujutsu's history model and Git storage, plus AVC-owned recovery, transactional coordination, and integration adapters. A fresh source-history engine is the fallback for a demonstrated mismatch, not the starting assumption.

This is a design recommendation. No comparative AVC performance experiment or production validation has been completed for the redesigned architecture.

## 2. Evidence discipline

Labels used below:

- **Documented:** A primary source describes behavior or an architecture.
- **Reported measurement:** The system's authors report a workload-specific result; not independently reproduced here.
- **Inference:** A conclusion drawn from those facts, not a measured result.
- **Decision:** The behavior proposed for AVC, not something the current code already does.

Sources are official documentation, implementation/API documentation, maintainer statements, and authors' engineering descriptions. Undated living documentation was consulted on the research date. Published figures are not predictions for AVC. Source listings at `latest` can change and even expose differently cached versions; implementation must pin a concrete source revision.

## 3. Existing systems: what to take and what not to infer

### Scion: scale boundaries, not a universal product template

**Documented, October 13, 2026:** Anthropic describes an online system with immutable changed-file records, derived query structures, server-side operations, sparse access, and tiered caches. Its deployment separates object creation from conditional branch updates and tests after merge, before deployment.

**Reported measurement:** The article reports more than 30,000 merged changes per day, roughly 100-fold faster checkouts, and a 200-fold reduction in its replacement fleet's CPU/memory footprint. These are internal comparisons against Anthropic's prior infrastructure, not a controlled comparison of general-purpose VCS designs.

**Inference:** Checkout-free APIs and independent caching/query layers are broadly useful. The server dependency, changed-file canonical representation, and post-merge testing policy do not follow for ordinary repositories. The article offers no generally available standalone Scion release.

Source: [Anthropic, Redesigning source control for a world of agents](https://www.anthropic.com/engineering/redesigning-source-control).

### Git: preserve interoperability and compare against a tuned baseline

**Documented:** Git snapshots share immutable trees and blobs; a new revision is not a complete physical copy. Pack/delta layout is distinct from logical identity. Conditional ref updates and ref transactions already exist.

**Decision:** Keep immutable snapshots as source truth. Do not attribute all tree, transport, or CI cost to the snapshot model. Benchmark shared local worktrees and maintained repositories, not only repeated full clones with default maintenance neglected.

Sources: [Git objects](https://git-scm.com/book/en/v2/Git-Internals-Git-Objects), [update-ref](https://git-scm.com/docs/git-update-ref), [worktree](https://git-scm.com/docs/git-worktree).

**Documented:** Partial clone omits promised objects and hydrates them later; it is distinct from sparse materialization. Commit-graph and bundle facilities address other costs.

**Decision:** Expose completeness and offline pinning. Compare history queries, transfer, materialization, and CI separately before proposing a universal replacement mechanism.

Sources: [partial clone](https://git-scm.com/docs/partial-clone), [commit-graph format](https://git-scm.com/docs/gitformat-commit-graph), [bundle format](https://git-scm.com/docs/gitformat-bundle).

### Jujutsu: the starting point for local history

**Documented:** Jujutsu has a library/UI split and replaceable storage interfaces. Its operation log records repository views, and its conflict model permits unresolved conflicts in revisions. Stable logical changes make amendments and rebases easier to follow than treating every replacement as unrelated work.

**Decision:** Reuse the history engine rather than recreate those semantics. Implementers must qualify concrete API behavior; an architecture diagram is not proof that an embedded application inherits all necessary durability or coordination guarantees.

Sources: [architecture](https://docs.jj-vcs.dev/latest/technical/architecture/), [operation log](https://docs.jj-vcs.dev/latest/operation-log/), [conflicts](https://docs.jj-vcs.dev/latest/conflicts/), [concurrency](https://docs.jj-vcs.dev/latest/technical/concurrency/).

**Documented:** The library exposes a transaction write that leaves an operation unpublished, an operation-head store boundary, and immutable repository views. These offer a plausible seam for one AVC transaction authority rather than two independently committed current states.

Sources: [library](https://docs.rs/jj-lib/latest/jj_lib/), [Transaction](https://docs.rs/jj-lib/latest/jj_lib/transaction/struct.Transaction.html), [OpHeadsStore](https://docs.rs/jj-lib/latest/jj_lib/op_heads_store/trait.OpHeadsStore.html).

**Documented:** Current `jj run` documentation includes isolated working copies, parallel jobs, amendment, and descendant handling. A parallel command runner is therefore not sufficient differentiation for AVC.

Source: [CLI reference, `run`](https://docs.jj-vcs.dev/latest/cli-reference/).

**Documented:** The Git compatibility page lists important unsupported or partial features, including attributes, hooks, submodules, partial clones, LFS, and some tag/shallow-repository behavior. Stable change metadata also cannot be assumed to survive arbitrary Git history rewriting.

**Decision:** Maintain an explicit feature matrix and fail before damaging unsupported repositories. Library reuse is not a license to advertise universal Git compatibility. Do not silently run a different content transformation or omit required checks.

Source: [Git compatibility](https://docs.jj-vcs.dev/latest/git-compatibility/).

### Sapling, Mononoke, and EdenFS: separate the axes of scale

**Documented:** Sapling's scale architecture evolved independent solutions for lazy file content, tree state, sparse workspaces, history queries, server operations, and virtual materialization. Commit backup is separable from deliberate public integration.

**Inference:** File count, historical content, commit graph size, and repeated workspace creation need different optimizations. An ordinary filesystem can remain the baseline while an optional lazy working-copy backend addresses extreme materialization cost.

**Decision:** Preserve checkout-free reads and interchangeable materialization boundaries. Do not make a sophisticated virtual filesystem or a large distributed service an installation requirement.

Source: [Sapling scale overview](https://sapling-scm.com/docs/scale/overview/).

### Mercurial and Gerrit: publication and intent are not new inventions

**Documented:** Mercurial distinguishes public, draft, and secret history phases. Gerrit associates revisions with a stable review/change identity and represents successive revisions as patch sets.

**Decision:** Separate editing, review, and integration. A stable identity is useful for continuity but must not transfer stale approval to different bytes. Preserve exact revision subjects and explicit publication boundaries rather than framing every private edit as shared history.

Sources: [Mercurial phases](https://www.mercurial-scm.org/help/topics/phases.html), [Gerrit Change-Ids](https://gerrit-review.googlesource.com/Documentation/user-changeid.html), [Gerrit patch sets](https://gerrit-review.googlesource.com/Documentation/concept-patch-sets.html).

### GitButler: composable work is not physical isolation

**Documented:** GitButler supports multiple branches and parallel-agent workflows within one working directory. Its maintainers have discussed the distinction between tracking separate changes and physically isolating files.

**Inference:** Shared-directory composition can be convenient for a cooperative human workflow. It does not prevent one worker from observing another's unfinished edits or running tests against mixed state.

**Decision:** Independent workers get isolated mutable workspaces by default. A future shared-directory composition mode must be explicitly cooperative, not presented as a sandbox or an isolation guarantee.

Sources: [parallel agents](https://docs.gitbutler.com/ai-agents/parallel-agents), [maintainer discussion](https://github.com/gitbutlerapp/gitbutler/discussions/12228).

### Pijul: independence and reusable conflict resolution

**Documented:** Pijul uses change/patch relationships rather than treating a change's identity as tied to one ancestry position; its channels and conflict-resolution model illustrate different ways to preserve independent intent.

**Decision:** Learn from explicit dependencies and reusable resolution provenance. Do not adopt patch algebra solely because it is different from snapshots. AVC still needs exact binary trees, straightforward validation subjects, efficient random access, and ordinary Git interchange. A convergent history representation alone cannot establish semantic compatibility of programs.

Sources: [Pijul rationale](https://pijul.org/manual/why_pijul), [channels](https://pijul.org/manual/workflows/channels).

### Fossil and SQLite: operational simplicity is a design feature

**Documented:** Fossil demonstrates a self-contained repository system with immutable artifacts and database-backed facilities. SQLite documents durable transactions along with their filesystem and synchronization assumptions.

**Decision:** A small local transactional metadata database is preferable to a bespoke storage stack until measurement shows otherwise. Keep immutable source payloads separate from frequently updated metadata, and distinguish authoritative data from rebuildable indexes. A metadata WAL is not equivalent to unnecessarily writing every source blob through two payload journals.

Sources: [Fossil technical overview](https://fossil-scm.org/home/doc/trunk/www/tech_overview.wiki), [SQLite atomic commit](https://www.sqlite.org/atomiccommit.html), [SQLite WAL](https://www.sqlite.org/wal.html).

### Perforce: accommodate non-mergeable assets

**Documented:** Perforce supports exclusive-open file policies for assets where simultaneous editing is not useful.

**Decision:** General-purpose does not mean source-text-only. Preserve binary identity and support optional server-enforced publication leases for appropriate paths. Do not impose locking on all ordinary code, and do not claim a disconnected worker can acquire a global exclusive guarantee.

Source: [Perforce exclusive checkout](https://help.perforce.com/helix-core/server-apps/cmdref/2026.1/Content/P4Guide/resolve.lock.exclusive.html).

### Mergiraf and Graft: useful structure without semantic overclaims

**Documented:** Mergiraf provides syntax-aware merging. Graft explores structural source entities and entity-level changes, with textual fallback and source-byte preservation as explicit concerns.

**Inference:** Structural information can reduce some unnecessary textual conflicts and improve review. Language parsing does not establish behavioral equivalence, side-effect independence, or correct combined intent. Invalid intermediate source, generated files, unknown languages, and binary content still need principled treatment.

**Decision:** Optional bounded merge drivers and derived semantic indexes; snapshots remain authoritative. No mandatory AST source database or automatic LLM resolver in the storage core.

Sources: [Mergiraf architecture](https://mergiraf.org/architecture.html), [Graft implementation](https://github.com/odvcencio/graft).

### Weft: examine agent-specific contracts critically

**Documented:** Weft presents intent-oriented work, capabilities, read tracking, evidence, and conditional integration, together with an alternative source representation. Its materials describe an early implementation and demonstrations, not independent proof of broad production behavior.

**Decision:** Scoped operations and exact evidence subjects are relevant prior art. They do not prove that all semantic conflicts can be detected, that prompt injection becomes harmless, or that trusted test authors cannot approve bad code. AVC should not claim these ideas are novel, nor require a source-text CRDT to use them.

Sources: [Weft project](https://weftgate.com/), [implementation](https://github.com/spranab/weft).

### Existing merge queues and build caches

**Documented:** GitHub's merge queue evaluates candidates including the target and earlier queued work. Bazel distinguishes action results from content-addressed outputs and requires suitable inputs for safe caching.

**Decision:** Reuse existing CI and host policy, preserve exact candidate identity, and avoid certifying a mutable checkout. A green run for an earlier base is not sufficient after a new combined candidate is formed. Do not characterize Git-based CI as inherently unable to validate exact revisions.

Sources: [GitHub merge queues](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/configuring-pull-request-merges/managing-a-merge-queue), [Bazel remote caching](https://bazel.build/remote/caching).

## 4. What agents actually change

**Inference, not a measured universal workload:** More independent workers and shorter edit/test cycles can increase abandoned alternatives, read fanout, workspace churn, and candidate integration rate. That makes several previously secondary costs important.

| Workload pressure | Useful response | What it does not solve |
| --- | --- | --- |
| Many local attempts | Bounded private recovery and cheap workspace reuse | Review quality or server load if every attempt is still pushed |
| Many independent writers | Separate mutable directories, exact API preconditions | External services, credentials, or process isolation |
| Stale assumptions | Pinned bases/dependencies and optional read observations | Complete semantic-conflict detection |
| Frequent remote updates | Quiet native drafts and deliberate proposal/land events | A host that triggers CI on every ordinary push |
| Large inspection volume | Checkout-free scoped queries and shared caches | Unbounded queries or poor cache authorization |
| Many candidate integrations | Exact manifests, queues, batching, cancellation, sound cache reuse | Free ordering or validation of one linear target |
| More automated output | Quotas, admission, artifact separation, review policy | Malicious or low-quality submissions by itself |

The new VCS should make these workflows correct and economical without assuming all code is agent-produced. Humans, IDEs, refactoring tools, dependency updaters, and CI can use the same contracts.

### Hosting-load claims that the evidence does not establish

Scion is strong evidence of one unusually large workload, not proof that agent commit count is the general cause of GitHub, GitLab, Codeberg, or Forgejo load. This research did not establish a causal cross-host breakdown. Do not retain that hypothesis as a verified project premise.

Measure local object writes, push/negotiation cost, refs, indexing, review activity, notifications, CI jobs, and artifact retention independently. Batching saves network and event overhead only where the host's behavior permits it. Faster commit construction does not automatically reduce CI compute, spam, or review effort.

## 5. Architecture alternatives

| Alternative | Strength | Consequential limitation | Decision |
| --- | --- | --- | --- |
| Git worktrees plus an agent-oriented CLI | Immediate host/tool compatibility, low new trusted code | Limited native history/recovery semantics; integration still needs explicit coordination | Baseline and simplest alternative to beat |
| Jujutsu engine, Git objects, AVC-owned coordination | Strong existing history model, incremental adoption, focused differentiation | API coupling and compatibility gaps; atomic adapter and retention still need engineering | **Selected implementation direction** |
| Fully native snapshot VCS with Git bridge | Maximum freedom over storage and semantics | Own object format, migration, correctness, bridge, GC, and ecosystem burden before proving value | Reconsider for a demonstrated blocker or workload gain |
| Scion-style online changeset service | Strong match for huge centrally operated monorepos | Mandatory service availability and substantial operational complexity | Optional scale inspiration, not general default |

Selection is not a score assigned to project prestige. The important question is which system best satisfies the target contracts at an acceptable lifetime cost. Jujutsu reuse remains preferable even when a few upstream improvements or narrow adapters are required. A permanent invasive fork or an inability to meet correctness contracts would justify revisiting that choice.

### Why not simply use Jujutsu?

That is the strongest alternative, not an objection to dismiss. Stable changes, undo, first-class conflicts, and parallel command execution are insufficient reasons for another product. AVC earns its existence only if its combination of bounded recovery, reliable worker handoff, transactional machine operations, transparent interoperability, and deliberate validated integration is materially better in complete workflows.

If the result is merely a renamed CLI, prefer contributing the useful components upstream or shipping a smaller companion. Do not build a new storage engine to manufacture differentiation.

## 6. Naming recommendation

**Recommend Tavi, command `tavi`, pronounced “TAH-vee.”** It is short, pronounceable, easy to use in commands such as `tavi diff` and `tavi land`, and not tied to a particular storage format or to AI. This is an editorial recommendation, not an applied rename.

The screen considered direct VCS and developer-tool overlap more important than unrelated uses of a short name:

| Candidate | Assessment |
| --- | --- |
| **Tavi** | Preferred sound and command ergonomics; no direct VCS collision found in the bounded search. Existing unrelated uses include a scientific visualization toolkit with a `tavi` entry point. |
| Vey | Compact alternative; spelling/pronunciation is less self-evident. |
| Trell | Branching association, but less distinct from existing application names and Trello. |
| Weft | Strong metaphor, rejected because a directly relevant VCS already uses it. |
| Graft | Strong VCS metaphor, rejected because a directly relevant VCS already uses it. |

Known Tavi overlap: [Triple Axis data Visualization Toolkit](https://github.com/neutrons/TAVI) and its [package configuration](https://github.com/neutrons/TAVI/blob/next/pyproject.toml). This screen is not a claim of global uniqueness, reserved package names, domain ownership, or legal clearance. The unrelated uses are not, by themselves, a reason to discard the name.

Keep `nijaru/avc`, the package/binary, and `.avc` unchanged until the name is selected. Apply a later rename coherently rather than leaving examples, executable names, and storage paths inconsistent.

## 7. Evidence still required from implementation

The architecture decision is made; these are proof obligations, not reasons to restart an unconstrained research project:

- Verify the pinned history engine can publish an unpublished operation through the single AVC authority, retain its object closure, and recover from interruption.
- Demonstrate bounded recovery without leaking private roots into normal export or retaining every snapshot forever.
- Exercise real Git feature requirements rather than assuming backend compatibility covers working-copy behavior.
- Compare complete human and multi-worker workflows against tuned Git and current Jujutsu, including failed attempts and remote/CI cost.
- Establish platform-specific filesystem and durability behavior before advertising support.

Do not carry unrelated storage-paper throughput figures into project targets. A mechanism's performance in a KV store, sync benchmark, or filesystem prototype is motivation for an experiment, not evidence of AVC performance.
