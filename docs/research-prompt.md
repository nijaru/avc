# AVC Research Prompt

I am restarting an early-stage project currently called AVC. It began as a Rust prototype for an agent-friendly, Git-compatible VCS with automatic local commits, squash/save points, an append-only operation log, and undo/redo. I am now reconsidering the design completely.

The broader problem I want to investigate is whether the recent growth of AI coding agents is creating meaningful infrastructure pressure on GitHub, Codeberg, Forgejo/Gitea, GitLab, and similar hosting platforms. Agents may produce many more commits, branches, pushes, pull requests, CI runs, webhooks, repository clones, and generated artifacts than traditional human workflows.

Research this as of today using current web sources. Do not assume that “agent commits” are the primary cause. Separate verified evidence from hypotheses and industry anecdotes.

## Questions to answer

### 1. What is actually creating load?

Investigate which operations are most expensive for hosting platforms:

- Number of commits
- Number of Git objects and trees
- Number of refs and branches
- Push frequency
- Pull requests and review comments
- Webhooks and integrations
- CI/CD runs and reruns
- Repository cloning and fetching
- Packfile generation and negotiation
- Garbage collection, repacking, fsck, and storage
- Repository indexing, code search, and browsing
- Notifications, email, and activity feeds
- API requests and rate limits
- Large generated files, binaries, logs, and artifacts
- Abuse prevention and spam detection

Distinguish client-side costs, Git server costs, database costs, CI costs, and general platform costs.

Look specifically for evidence involving AI coding agents, automated commit generation, Dependabot/Renovate-like automation, mass branch creation, and high-frequency repository activity. Search GitHub, Codeberg, Forgejo, Gitea, GitLab, Git mailing lists, engineering blogs, issue trackers, and relevant conference or academic sources.

### 2. Does commit volume itself matter?

Explain when a large number of small commits is expensive and when it is relatively cheap.

Compare:

- Many commits touching the same small files
- Many commits producing new trees
- Many branches and refs
- Many pushes containing one commit each
- One push containing many commits
- Many CI-triggering pushes
- Local commits that are never pushed
- Squashed commits
- Bundled or batched updates

Discuss Git’s object model, packfiles, deltas, refs, reflogs, garbage collection, fetch negotiation, and server-side indexing. Use authoritative technical sources and avoid unsupported claims.

### 3. Could a new VCS reduce this pressure?

Evaluate whether a new VCS could materially reduce infrastructure load, or whether the main solutions belong in hosting platforms and CI systems.

Consider designs such as:

- Local automatic savepoints that are not immediately published
- Explicit publish/checkpoint operations
- Batching many local changes into one remote update
- An append-only local operation journal
- Content-addressed storage
- Snapshot-based history
- Chunked or Merkle-tree storage
- Lazy materialization of trees
- Semantic commits versus raw filesystem snapshots
- Server-side journals that materialize Git-compatible history only when needed
- Bundled synchronization protocols
- Deferred or coalesced webhooks and CI triggers
- Separate “draft history” from “canonical shared history”
- Local-first or peer-to-peer synchronization
- Git compatibility as an export/import layer rather than the core storage model

For each design, explain what load it reduces, what load it does not reduce, and what new complexity or failure modes it introduces.

### 4. What is state of the art?

Compare current systems and relevant technologies:

- Git
- Git partial clone, sparse checkout, commit-graph, multi-pack-index, bundle URIs, protocol v2, and related optimizations
- Jujutsu/jj
- Mercurial
- Sapling
- Pijul
- Fossil
- Gitoxide
- libgit2
- Dolt, if relevant
- Distributed or content-addressed systems
- Existing source-control systems designed for monorepos or high-scale development
- Any newer systems specifically addressing AI-generated code or high-frequency automated changes

For each, compare:

- Local commit/save performance
- Large-history performance
- Large-file performance
- Branching and merging
- Storage efficiency
- Push/fetch efficiency
- Concurrency and collaboration
- Conflict handling
- Undo and rewriting
- Hosting/server requirements
- Human UX
- Agent/API UX
- Ecosystem maturity
- Git interoperability

Do not call something “state of the art” without explaining the metric and citing evidence.

### 5. What should AVC become?

Based on the evidence, propose 2–4 viable architectures for a new VCS.

At least consider:

1. A Git-compatible client focused on local automatic saves and remote batching.
2. A new snapshot/journal-based VCS with Git import/export.
3. A local-first VCS with explicit publish checkpoints and server-side materialization.
4. A high-performance conventional VCS optimized for large histories and automated workflows.

For each architecture provide:

- Core data model
- Storage format
- Local history model
- Remote synchronization model
- Branch and merge model
- Undo/recovery model
- Compatibility strategy
- Expected performance advantages
- Main trade-offs
- Difficult implementation problems
- Which hosting infrastructure would need to change
- A realistic MVP scope

Be especially critical about whether a new client-side VCS can help if hosting platforms still receive one push, one webhook, and one CI run per agent action.

### 6. Recommend a benchmark plan

Design a reproducible benchmark suite comparing Git and candidate designs.

Include workloads such as:

- 100,000 tiny commits
- 1,000,000 tiny local savepoints
- Many commits touching the same files
- Many commits touching different files
- High branch/ref counts
- Frequent push versus batched push
- Large monorepo checkout
- Large binary files
- Generated files
- Merge-heavy history
- Agent-style edit/test/fix loops
- Multiple concurrent agents
- Clone, fetch, log, status, checkout, merge, and garbage collection

Define:

- Dataset sizes
- Hardware and filesystem requirements
- Metrics
- Warm versus cold cache
- Local versus server-side measurements
- Storage size
- CPU and memory
- Latency
- Throughput
- Network bytes
- Number of webhooks and CI jobs
- Reproducibility requirements

Identify existing benchmarks before inventing new ones.

## Required output

Produce a research report with:

1. Executive summary
2. Evidence about what is actually overloading systems
3. Distinction between commit volume, push volume, CI volume, and storage/object volume
4. State-of-the-art comparison table
5. Feasibility analysis for a new VCS
6. 2–4 architecture proposals
7. Recommended direction for AVC
8. Benchmark plan
9. MVP roadmap
10. Risks and reasons not to build a new VCS
11. Open questions requiring further research

For every important factual claim, include a source link and publication date where available. Prefer primary sources: official engineering posts, technical documentation, source repositories, design documents, mailing-list discussions, issue trackers, academic papers, and measured benchmarks.

Clearly label:

- Verified fact
- Measured result
- Reasonable inference
- Speculative design proposal

Do not optimize only for raw commit speed. Evaluate total system cost, synchronization, hosting load, CI behavior, collaboration UX, recovery, interoperability, and operational complexity.
