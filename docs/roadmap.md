# AVC evaluation and implementation roadmap

**Reviewed:** 2026-10-14. **Design:** [design.md](design.md). **Evidence:** [research.md](research.md).

The existing Rust 0.0.1 implementation remains a Git wrapper; this investigation does not change or qualify it. The reference snapshot-oriented candidate is not a final canonical-format decision. Do not implement the previous State/WorkVersion/Application hierarchy merely because it appeared in an earlier design.

## 1. What has actually been checked

[Contract probes](experiments/vcs_contract_probes.py) and [recorded results](experiments/vcs_contract_results.json) reproduce five narrow cases on Linux, Python 3.13.5 and Git 2.47.3: identical deletion versus causal retraction; disjoint edits breaking a combined program; a restore check/replace race; prepared-object GC races in a small state model; and unnecessary global-generation conflicts.

Run:

```sh
python docs/experiments/vcs_contract_probes.py --output results.json
```

They are research fixtures, not a VCS benchmark or proof of crash safety. Passing means the counterexamples were observed. No native engine, current Jujutsu/Pijul runtime comparison, user trial or power-loss qualification has been completed.

## 2. First useful implementation, not three entire VCSs

Implement the concrete Candidate A local loop in the existing Rust project: immutable files/trees/revisions, resource generations, explicit capture, history/diff, merge with typed conflicts, and safe restore to a separate directory. A transactional local reference backend should keep objects and root transitions under one authority before optimizing storage.

Keep the production model separate from a small independent oracle. Before any stable on-disk format, test parent/source interpretation, amendment divergence, split/fold lineage, resource-scoped CAS, retry outcomes and retention reachability. Test `integrate`, content `revert`, explicit `reapply`, and local `undo` as different operations. Do not use one eternal ever-applied ChangeId set to reject legitimate reapplication after a revert.

Preserve old .avc data before any migration. Replace obsolete Git-wrapper paths only when the native replacement works; do not attach new atomicity claims to the separate Git/JSON operations.

Acceptance: reopen exact retained bytes; no partially selected resource transitions; deterministic conflict output; stale requests do not overwrite current roots; private captures do not move public names; recovery into a new directory preserves the original. Keep in-place uncontrolled-writer limitations explicit.

## 3. One bounded comparison for each consequential bet

### History semantics

Compare the same histories in the reference snapshot model and an existing Pijul/Darcs-style implementation or a faithful small causal interpreter. Do not write a toy anchored insertion structure and describe it as a complete patch calculus.

Cases: concurrent identical deletes and selective undo; same-position insertion ordering; move/edit and copy/edit; delete/recreate; binary replacement; a change reverted and intentionally reapplied; dependency v1 versus v2; repeated reapplication after conflict resolution; long-offline divergence; effects introduced by ordinary atomic-save editors rather than controlled APIs. Report exact retained intent and conflicts, not just final byte equality.

Decision: prefer graph B only for demonstrated semantics worth its capture/materialization/retention costs in target workflows. A source-state system may need richer provenance to match those semantics; count that cost fairly.

### Snapshot commitment versus deferred materialization

Use a common object/query boundary to compare snapshot roots against parent-relative changed-file records plus indexed/checkpointed materialization. Measure admission, cold and warm historical read, first checkout, full index rebuild, GC and offline pinning. Keep source bytes and durability level identical. This experiment need not reproduce all of Scion's distributed infrastructure.

Decision: adopt C when admitting rarely consumed drafts materially improves the complete workload after accounting for first-read and repair costs, not solely its append microbenchmark.

### Physical storage and large directories

Start from the reference transactional store. Build only the measured necessary alternatives: one object/root journal or staged immutable segments with metadata authority. Compare fsync level, objects and resource transactions fairly. Test disk-full, truncated records, wrong hashes, concurrent staging/GC, long readers, checkpoint pressure and interrupted compaction.

Compare flat hierarchical directories with a deterministic paged candidate on ordinary trees and deliberately wide 10/1,000/100,000/1,000,000-entry cases. Check insertion-order identity within the specified format, page-boundary shift behavior, small-repository overhead, rename and sparse diff. Do not freeze the canonical paging/chunk layout while calling it a physical implementation detail.

## 4. Complete human and agent workflow evaluation

Use maintained upstream Git with appropriate worktree, maintenance, commit-graph and sparse/partial features; pinned current Jujutsu; relevant graph baselines; and AVC. Record versions and supported features. The installed Git 2.47.3 used for contract probes is not the future performance baseline.

Include three different agent modes: sequential edit/test/capture; competitive alternatives where one is chosen; compositional parallel tasks where several are integrated. Sweep 1, 8 and 32 workers as experimental conditions, not claimed typical population counts. Include external services and build-cache interference when a test purports to measure sandbox isolation.

Measures: task success, source-loss incidents, recovery success, stale retries, wrong auto-merges, unnecessary conflicts, review work, p50/p95/p99 latency, CPU/memory, network and disk bytes, retained space after collection, offline availability, CI compute and notification count. Include failed attempts and maintenance. A faster automatic wrong merge is a regression.

Adapt GitGoodBench cases to native task outcomes with equal model/tool budgets. Do not reward either CLI for imitating a particular command sequence. Use human trials for state understanding, first-use errors, interrupted-work recovery and experienced efficiency. Current research supplies requirements and cases, not a measured population survey.

## 5. Durability and filesystem qualification

Use independent state-machine/property tests for transactions and generation checks. Fault-inject each persistence stage. Distinguish process crashes, write failures and actual power-loss assumptions. Restore authority and immutable closure together; verify objects, not just row counts. Use supported APFS/Linux/NTFS configurations explicitly rather than extrapolating from one container.

Test the reproduced restore race. An API token or an immediately preceding content check is not writer exclusion. Guaranteed recovery must use a new destination or an actual controlled-writer boundary. Before-images protect what was observed; they do not magically preserve writes made after their capture.

Retention tests must include expiring captures, named pins, published history, in-flight imports, readers and segment/graph references required by old retained revisions. A causal graph must demonstrate what happens when a client returns with references older than a pruned epoch.

## 6. Git exchange and native collaboration

Qualify import/export early on real source/mode/history fixtures, and move the adapter into ordinary use once the local loop is safe. Preserve unchanged original objects where promised and expose native metadata loss. Cover attributes/EOL/filters, symlinks, names, signatures/tags, submodules, LFS and shallow/partial history. Unsupported operations fail explicitly before alteration. Avoid live dual-writer colocation initially.

Then add optional restricted draft sync and a minimal target service. Transfer immutable objects in batches, validate closure/authorization, keep uploads protected until admission, and make target CAS plus policy/receipt/outbox one authority transaction. Test revoked authority during validation, stale evidence, post-commit lost replies, retries and duplicate events.

Existing Git hosts retain their own guarantees. Use their actual protection/merge queues and reconcile uncertain outcomes. Native capture must not be translated into a public push per editor change.

## 7. Exit criteria for architectural selection

Select a canonical model and stabilize its format only after the competing mechanism has been evaluated on the requirements where it plausibly wins. The whole project need not wait for every optimization or a complete enterprise service.

Hard gates are source integrity, explicit conflict/undo semantics, trustworthy capture/restore, supported interoperability, and no silent authority bypass. Performance gates come from recorded baselines, not invented speedup multiples. A feature earns inclusion when it improves important full workflows without unacceptable ordinary-project regressions.

Keep naming separate; Tavi is not approved. Do not rename packages, directories or versions during this design work.

For future Rust implementation changes, run the project gates in addition to the relevant scenario checks:

```sh
cargo test
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```
