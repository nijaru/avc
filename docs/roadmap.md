# AVC implementation roadmap

**Direction established:** 2026-10-13  
**Architecture owner:** [design.md](design.md)  
**Evidence and alternatives:** [research.md](research.md)

Deliver working vertical slices. The intended product is not implemented by the current Rust command wrapper, and the first milestone is not a custom segment store or a distributed server.

## 1. Starting point and migration

The source baseline inspected for this redesign is commit `ce035bc3829d77646bc07e6db2cd787f8ce9620b`.

The existing package is `avc` 0.0.1. `src/git.rs` executes Git commands, stages all changes, uses short hashes in helpers, and exposes soft/hard resets. `src/oplog.rs` appends JSON records separately without a durability flush or a shared Git/metadata transaction. `src/commands/save.rs` performs source-history mutation before recording its log entry. These paths do not provide the new architecture's atomicity or concurrency model.

Do not layer the new guarantees onto those command sequences. Replace the affected command loop and state ownership as each vertical slice lands, removing superseded helpers and tests with it. Git history retains the old implementation; a parallel `legacy` engine is not needed. Preserve useful command parsing, output, and fixtures only where their contracts remain appropriate.

No version or executable rename is part of this design update. Do not claim existing `.avc` JSON logs are the new format. Before an implementation changes the on-disk format, detect old repositories and offer an explicit preservation/import path; do not silently reinterpret them or automatically reset their Git histories.

## 2. First milestone: a real transactional repository

Build the smallest useful library-backed repository loop in the existing Rust crate: initialize/import a supported Git repository, read status/diff/log, capture a tree, create a revision, and restore it safely.

### Implementation decisions

Use a pinned `jj-lib` source/release and its Git backend. Use SQLite for AVC metadata and active source-operation authority. Introduce one repository adapter, not a general plugin registry or a second source DAG. Establish reusable seams for CLI and future machine/server clients.

The first vertical slice must exercise unpublished operation creation and coordinated publication. Verify actual dependency source behavior for fsync, object closure, operation-head ownership, GC, and cancellation. Upstream or adapt a narrow missing seam rather than committing through two independent current-state stores. A reproduced fundamental mismatch requires an explicit design revision, not silently weakened guarantees.

Keep the module boundaries concrete: repository/history adapter, authoritative state transactions, working-copy/recovery operations, and CLI/API presentation. Split modules as responsibilities become real; do not pre-create a workspace of speculative crates.

### Acceptance

A source mutation is visible with its complete metadata or not visible. Crash/fault injection before object durability, before root commit, and after commit/before reply preserves that contract. Retrying the same accepted request returns its original outcome. Retired request epochs cannot be reused to turn expired retries into new writes. A failed request does not move a public Git ref.

Demonstrate restoration after interrupted materialization without overwriting a newly edited file. Exercise paths outside the process working directory; no command should implicitly mutate whichever repository happens to be the subprocess CWD.

Qualify a concrete ordinary Git repository fixture: exact file bytes, modes, names, parents, and imported object IDs. Detect unsupported features before a mutation. The milestone is foundation code for the working product, not a disposable performance spike.

## 3. Second milestone: usable local changes and workspaces

Add named logical changes, amend/split/fold, source operation undo, deterministic merges, structured conflicts, and multiple workspaces sharing the store. Keep the common solo workflow usable without a daemon or server.

### Acceptance

A change retains identity under amendment/rebase; divergent amendments are visible rather than last-writer-wins. Splits/folds record understandable lineage. Restoring source state does not resurrect expired authority or replay external effects.

Two workers can edit, inspect, and test independently. A stale workspace generation is rejected. Rewriting a dependency does not silently rewrite another active worker's directory. Reacquiring or transferring a workspace invalidates stale managed credentials and preserves the old directory when a process may still be writing.

Merge tests cover deletion/modification, file/directory type changes, renames where supported, binaries, nested prior conflicts, and deterministic resolution. Keep tests focused on source and operation outcomes, not private call sequences or incidental CLI prose.

## 4. Third milestone: bounded recovery and the machine API

Add automatic observation and private recovery snapshots outside semantic revision churn. Implement admission rules, visible capture status, retention budgets, named pins, safe GC, and the versioned noninteractive API. A watcher/daemon is now justified by a real continuous-capture workflow, not by architectural preference.

### Acceptance

Tracked/admitted edits become recoverable, no-op saves coalesce, and ignored/unadmitted files are not silently retained. Watcher overflow forces reconciliation. Continuously changing files, unreadable files, disk exhaustion, and unsaved editor buffers yield honest coverage/status rather than a false protection claim.

Repeated saves reach the configured retention behavior after collection. Live readers, validation candidates, pins, imports, and retained operations survive collection. An interrupted GC or backup produces a valid old/new state, not a missing referenced object. Test privacy of default export refspecs and reachable closure.

API operations enforce exact revision/generation preconditions, stable request outcomes, bounded reads, cancellation semantics, and immutable pagination. Compare expected file hashes before applying all edits atomically. Checkout-free reads and edits must use the same source semantics as filesystem-backed commands.

A useful end-to-end example is one person and several workers performing edit/test/fix cycles, recovering from a killed process, selectively combining work, and discarding unsuccessful alternatives without public history noise.

## 5. Fourth milestone: existing-host collaboration

Implement clone/fetch and selected revision export, proposals, review identity mapping, exact validation subjects, and integration through an existing Git host's authoritative policy. Start with one complete adapter rather than superficial support for every forge.

### Acceptance

Two participants can collaborate with one using ordinary Git tools. Unchanged history is preserved; rewrites and metadata loss are explicit. Unresolved native conflicts cannot accidentally be published as normal source. Raw prompts and automatic recovery roots do not enter ordinary pushes.

Build and maintain the compatibility matrix from actual fixtures. Attributes/EOL/filters, submodules, LFS, signed objects, tags, and shallow/partial clones need positive support tests or a precise early refusal. No invocation may imply an unsupported hook or policy check executed.

For integration, demonstrate that stale base, changed proposal, changed policy, revoked authority, or evidence for a different candidate cannot yield a false verified success. Candidate digests exclude their later evidence; acceptance and receipts refer to immutable subjects without cycles. Exercise network loss after a host accepts a request and reconcile the actual result without duplicating the action. Use host merge queues/protection where needed; do not pretend a client-only check enforces remote policy.

At this point, compare the whole product with Jujutsu plus existing Git-host tools. If the improvement is merely command spelling, prefer a companion or upstream contribution over expansion into another platform.

## 6. Fifth milestone: optional managed collaboration

Only after the local tool and existing-host path work, add restricted quiet draft sync and a small managed proposal/target service. One service/database plus immutable object storage is sufficient initially; do not require an issue tracker, CI scheduler, distributed cache fleet, or new review UI.

### Acceptance

Uploads stay quarantined until their authorized object closure is valid. Duplicate uploads and retried manifest requests are safe. No quiet draft transition emits a public review/indexing/CI event unless explicitly configured.

A managed landing atomically verifies target/policy/authority, selects the exact candidate, records its receipt, and enqueues its event. Delivery can repeat; consumers deduplicate by event ID. Independent targets do not suffer unnecessary logical CAS failures because an unrelated target advanced.

Exercise batch validation, superseded jobs, unfair queues, cancellation, policy revocation during tests, and replayed attestation/request IDs. A green batch certifies the stated candidate only, not every intermediate revision or deployment.

Document backup/restore, key and permission management, retention, quota enforcement, and object-store failure behavior. A service that needs undocumented operator intervention to recover is not complete.

## 7. Scale features: add in response to evidence

The mature interfaces allow lazy source reads, sparse/virtual materialization, remote query acceleration, stronger asset handling, paged trees, and a native backend. Do not schedule all of them as mandatory stages of product completion.

Adopt a feature only after identifying the limiting workload, comparing a simpler baseline, and measuring complete costs. Examples:

- Too much repeated checkout I/O: compare shared stores, sparse materialization, reflinks, and lazy workspaces before a custom filesystem.
- Large directory update cost: compare current Git trees against a deterministic paged map, including object count, lookup, diff, transfer, and bridge cost.
- Recovery write amplification: compare bounded batching and existing packing against a native append path before choosing a new canonical format.
- Large asset churn: qualify LFS or chunking, explicit conflicts, and optional fenced publication leases.
- Read fanout: batch scoped queries and add bounded cache tiers with authorization and failure isolation.

The history/storage adapter may evolve without changing the product model. Avoid freezing a generic backend framework around mechanisms that have not earned a place in the implementation.

## 8. Benchmark and verification plan

### Baselines and provenance

Compare current, pinned Git with maintenance/commit-graph and appropriate worktree/sparse options; current, pinned Jujutsu; and AVC. Compare native mechanisms only on the operations they actually implement. Scion's internal measurements cannot serve as a reproducible local baseline.

Record source versions, exact commands/configuration, fixture generator seed, hardware, filesystem, storage media, cold/warm cache procedure, worker count, and raw results. Separate foreground latency, amortized background maintenance, and full lifecycle cost. Report failures and retries, not only successful fast paths.

### Workloads

| Workload | Sweep | Essential measurements |
| --- | --- | --- |
| Solo edit/inspect/recover | Small project, moderate monorepo; clean/dirty; no-op and changing files | Latency distribution, bytes read/written, retained space, capture lag |
| Frequent recovery | Same/different files; rising save counts through at least one retention cycle | Live versus reclaimable bytes, GC cost, recovery time, ref/object growth |
| Concurrent workers | 1, 8, 32; disjoint, overlapping, cross-file dependent edits | Throughput, p50/p95 latency, stale retries, interference, resource sharing |
| Tree shape | Many directories, very wide directory, moves, sparse differences | Update/diff/lookup cost, object count, transfer/read amplification |
| Inspection | Log, blame, diff, file/range reads, repeated agent queries | Cold/warm cost, cache hit rate, bytes, bounded-query behavior |
| Integration | Serial versus batch; superseded proposals; target/policy changes | Validated changes per minute, queue time, discarded work, CI compute |
| Collaboration | Local-only, draft batches, individual/batched proposals, retry/loss | Server CPU/bytes, transactions, webhooks, review events, CI jobs |
| Large assets | Whole replacement, small edit, non-mergeable conflict | Storage/network cost, retrieval latency, lease correctness |

Use both ordinary source repositories and controlled fixtures. Include at least one semantic-interference example with disjoint modified files. Do not infer semantic independence from a low textual conflict rate. Increase history/save counts to expose asymptotic behavior, but do not require million-event tests on every unit-test run.

### Fault and trust matrix

Exercise process termination and injected storage failure around object persistence, pinning, metadata commit, workspace replacement, retention, upload acceptance, target CAS, and event acknowledgment. Include concurrent source mutation, stale lease takeover, duplicate requests, missing promised objects, path collisions, symlink traversal, malformed objects, and untrusted executable configuration.

Use a small reference model and property-based tests for state transitions where they reveal otherwise hard-to-reproduce corruption. Filesystem and storage assertions must match the actual platform's durability contract. Distinguish process-crash tests from real power-loss guarantees.

### Success criteria

Correctness contracts are release gates. Performance targets should be set from published baseline runs, not arbitrary multipliers chosen in advance. A mechanism succeeds when it improves a relevant complete workflow without unacceptable regressions in ordinary use, correctness, operational burden, or Git interchange.

Run the crate's prescribed checks for implementation changes:

```sh
cargo test
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```

Passing tests is evidence for the cases exercised, not proof that the entire design is implemented or safe. Keep the compatibility and supported-platform claims aligned with actual end-to-end qualification.
