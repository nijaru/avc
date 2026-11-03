# AVC — qualification and implementation plan

**Updated November 3, 2026.** This is the active execution plan for [the working architecture](design.md). It replaces the old reference-model-first milestone sequence; use the repository's Git history to recover superseded detail when investigating a specific old experiment. Design refinement does **not** authorize silently admitting irreplaceable user data.

## Current verified boundary

As described by the current source tree and README at this review, `src/objects.rs`, `src/merkle.rs` and `src/verify.rs` implement portions of the **previous** draft codec, deterministic in-memory map construction, streaming BLAKE3 hashing and closure verification. `src/main.rs` has informational flags only; operational CLI commands refuse. Tests and independent Python/Node fixture models exercise narrower properties; they are not a durable repository, native clone, source recovery, or release qualification.

**Do not equate the code with the November 3 architecture.** Reconcile the codec and its fixtures deliberately after making the open canonical decisions. Keep existing Git repositories and old `.avc` recovery data intact; never import, rewrite, discard or auto-clean them as part of a design reset.

## Workstream A — settle hard choices using real implementations

Run a **short, bounded** comparison for the decisions with highest migration cost. Do not design a new abstract framework for every contestant.

| Decision | Candidates | What decides it |
| --- | --- | --- |
| Canonical source structure | Compact sorted/hierarchical directory map; path-compressed radix; deterministic chunked B-tree | Incremental edit, cold lookup, clone metadata, directory moves, memory, byte churn, verification |
| Merge/conflict | Practical versioned three-way driver + typed alternatives; normalized/flattened conflict expressions | Losslessness under repeated rebase/revert, ordinary conflict burden, binary/type/delete cases, resource bounds |
| Payload store | SQLite-only, or SQLite transactional roots + immutable segments | **Same** crash semantics; ingest, read, compaction, retention, backup, SSD/RAM cost |
| Snapshot vs changeset authority | Snapshot-root native history versus incremental authored changeset/derived tree model | End-to-end commit/merge/query/clone costs and offline correctness with wide repos, not prestige of an upstream system |
| Clone/Git bridge | Native selective fetch model; Git partial-clone adapter | Time to first usable source; repeated fetches, deepening, missing-object accuracy, interoperability |

Use existing fixtures and research where relevant, but run **new native timings** and inspect data with realistic workloads: small Rust/Python projects, wide directories, deep history, large binary files, repeated edits and merges, and many independent agents. Include cold and warm caches, constrained memory, CPU time, network bytes, system calls, disk use, tail latency, and operational complexity.

**Exit:** one explicit proposed layout/format/storage decision with reproducible evidence. If experiments are inconclusive, prefer simpler verified correctness over speculative speed. The first practical implementation may begin before every performance question is resolved, but **freeze no canonical encoding or long-lived storage promises before relevant safety tests**.

## Workstream B — first trustworthy native vertical slice

Implement in Rust with a reusable library and thin CLI. Progress through end-to-end behavior, not a larger suite of disconnected reference experiments:

1. Define and independently verify the selected canonical object bytes/IDs, typed source entries and persistent tree edits. Replace experimental codec/fixtures together; do not introduce compatibility shims for an unreleased format.
2. Admit objects with bounded validation and explicit full-closure protection. Implement atomic local metadata/root transactions with crash-safe staged payloads under the chosen physical backend.
3. `init`, `status` (read-only), immutable `show`/`diff`, local commit from pinned selected filesystem content, basic `log`, export, backup and verify.
4. Implement an operation receipt and protected before-images for guarded checkout/restore. Validate recovery of interrupted operations and exact scope before adding convenience navigation.
5. Add full source-tree and revision merges, explicit typed conflicts, and versioned text driver after workstream A's semantic evidence.

**Custody gate:** after power-loss/fault injection at every durable write boundary, metadata and objects are either fully valid and reachable or safely unacknowledged; never expose an acknowledged root with missing closure. Exercise GC against concurrent readers and protected pins; independent backup/export restores exact bytes without internet. Corrupted hashes, forged IDs, truncated segments, symlink traversal, path collisions, out-of-scope deletes and unknown source coverage must not produce false success. Until then, commands must clearly state experimental/non-custodial status or refuse important-source operations.

## Workstream C — fast useful clones and Git adoption

This is a **high-priority product slice**, not postponed until hosting.

### Native selective clone contracts

Implement object promises and history horizons as separate features:

- Fetch exact selected remote ref/tip, authenticated tree metadata and a useful source working set; allow `--ref`, `--paths`, `--no-checkout`, `--history=all` and `--full` (illustrative spellings).
- Choose normal materialization for small projects; for huge repos allow sparse scope and/or optional virtual filesystem without fabricating files. A CLI/agent can read an immutable remote file without checkout.
- Support bounded parallel/batched fetch, resumable object acquisition, request coalescing, scope expansion, and offline hydration.
- Distinguish promised/missing, inaccessible, truly absent, corrupt and complete/verified. No use of unknown ancestry as an empty merge base. Publish only through a full, authorized holder or with complete local proof.

Initial Git access may use a `git` subprocess and Git partial-clone features while AVC normalizes supported source into native IDs. Avoid an unconditional depth-one Git clone on developer workspaces because it can make subsequent fetches more costly; select measured policies for ephemeral source-only jobs versus long-lived local work.

### Git round trip

Implement Git branch import and remotes early, after basic native source read/commit:

- Preserve a persistent foreign SHA/ref mapping and accurate upstream tracking; avoid reimport duplicates.
- Fetch incremental upstream changes; resolve and expose rewrites, altered refs and missing history honestly.
- Export supported native files/commits and conditionally push to a Git remote. Test repeated round trips and independent Git-side updates.
- Report unsupported representations and lossy conversion *before* effectful updates; never materialize unresolved source conflicts as ordinary clean Git files.
- Explicitly test LFS pointers, submodules, unusual path bytes, executable/symlink handling, signed tags/commits, merge parents, empty directories, and shallow Git boundaries.
- Keep a single writer for each working directory; evaluate optional Git colocation only after correct non-colocated sync.

**Exit:** a developer can `avc clone` an existing GitHub project, edit and commit natively, fetch upstream, and safely publish a representable change back to Git, with no Git object format in AVC's canonical source authority.

## Workstream D — history ergonomics and concurrent agent work

Once the custody and first Git onboarding flows are credible:

1. Stable logical change references; amend, rebase, restack, split/fold and scoped operation-log undo with correct retention of originals.
2. Multiple independent/stacked draft heads and reproducible **composed source views**; explicit assignment of selected file/hunk edits; visible dependency/conflict graph.
3. Isolated writable workspaces for multiple concurrent agents, plus checkout-free conditional source edits; reject stale expected bases and unexpected overwrites.
4. Support review of one stack or a composed integration candidate without prematurely committing synthetic merges. Derived composition never silently changes each stack's canonical revision.
5. Qualify failure recovery, cancellations, repeated rebases, conflicting patches and same-file independent hunks with real agent/editor/CLI workflows.

Usability tests must include common Git tasks (fix and commit, split a mistaken change, switch with dirty work, recover after interruption, rebase, resolve conflict) and agent tasks (several independent changes, same-file collision, sparse read, integration check, share back to Git). Test whether change stack semantics solve more problems than they create before adding automatic attribution.

## Workstream E — native remote and hosting scale

After local plus Git end-to-end value exists, design and implement the native remote service against the same core object semantics:

- Immutable object reads and batch transport, selective fetch, efficiently reusable transfer groups, caches and independently derived indexes.
- Per-ref conditional publication; idempotent retry; clear consistency when reading multiple refs; verified durable object closure before acknowledgment.
- On-demand source queries without checkout, delegated validation for thin clients, access/authorization-safe responses.
- Benchmark independently scaled storage/metadata/query services where appropriate. Apply Pando's per-ref coordination and transport reuse, and Scion's cache/derived-index ideas where observed throughput warrants them.
- Favor a single code path for local and remote source semantics; the hosted service may use different physical storage and coordination primitives.

## Criteria for releasing AVC as a trustworthy VCS

No stable release or irreplaceable-source custody until:

- **Integrity and recovery:** durable acknowledgments, forced interruptions, competing writers, GC/compaction, secure paths and exact restore/export pass adversarial native checks on macOS, Linux and supported Windows paths.
- **History semantics:** merge/undo/rebase with unresolved conflicts conserve protected inputs and never imply false correctness or silently lose history.
- **Promise semantics:** thin clones give honest unavailable/unknown outcomes; full native exports and full clones are independently reconstructible.
- **Interoperability:** Git import/push round-trip covers the explicitly supported subset; refused/lossy cases are precise.
- **Usability:** ordinary CLI work, partial clone expansion, change organization and recovery are exercised from the real command entry point rather than model-only tests.
- **Performance:** representative end-to-end task/clone/store cost against Git/Jujutsu/GitButler and relevant repository sizes is measured without extrapolating upstream vendor metrics.
- **Format:** independent cross-language or second-implementation vectors agree on untrusted decoding, canonical encoding, type/ID validation, and limits; versioning and migration strategy is documented.

## Next local/device session

1. Enter `~/github/nijaru/avc`; inspect worktree, remotes, current branch and recent commits before any changes. Read `nijaru/dotfiles/instructions/AGENTS.md`, this document, [design](design.md), [UX](ux.md), [research](research.md) and `nijaru/knowledge/projects/avc/constraints.md`. Do not discard concurrent local work.
2. Evaluate workstream A's **three highest-risk choices** on source cases already in research: canonical map layout, normalized conflict model/merge driver, and durability-equivalent payload storage. Expand the sample where biased. Record results with exact inputs and commands.
3. Reconcile the experimental Rust codec with the selected new canonical model. Remove outdated prototype assumptions and update the fixtures as a cohesive migration, not multiple half-compatible formats.
4. Implement the first actual native local vertical slice and run focused compile, corruption, crash and workflow checks. Do not resume the disabled Git wrapper as source authority.
5. Start Git adapter experiments and the tip-first object-promise contract as early as the local engine permits. Review performance and UX after **real** source-to-Git tasks, then advance the hosted protocol only if justified.
6. Update the single maintained owners rather than adding another handoff or parallel specification. Push coherent task-owned code/doc changes when verified; no PR or external publication is implied.

Avoid arbitrary projected schedules and test-count quotas. **The next milestone is a trustworthy, measurable local-plus-Git workflow, not a larger formal plan.**
