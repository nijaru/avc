# AVC — authoritative architecture

**Design baseline: November 4, 2026. Ready for local implementation, not a released or validated format.** This file owns goals, authority boundaries and non-negotiable architectural decisions. The **normative detail is deliberately split by ownership**: [canonical objects/merge](format.md), [local durability/recovery](storage.md), [native sync](sync.md), [foreign Git adapter](git.md), [user/API workflows](ux.md), [implementation and verification](roadmap.md), and [adversarial rationale](research.md). A future agent must read the relevant owner, not apply an old design from chat or Git history.

The November 2 and November 3 implementations/specifications were *unreleased prototypes*. Their code and ~187 KB research corpus are historical evidence in [Git history at af66a638](https://github.com/nijaru/avc/tree/af66a63898f383b596e4828199dc625a80e28a72). Nothing there is a compatibility contract. New format objects should have **new IDs and fixtures** rather than shims to obsolete kinds.

## 1. Product definition and target workflows

**AVC is a native, local-first, general-purpose version-control system**, designed to make exact source state, editable history, interrupted work, and concurrent agent-produced changes easy to inspect and manipulate. It must be credible for an ordinary laptop and offline editor **without** a service or daemon, and also support sparse metadata-first API work in large remote repositories, transient containers and CI.

Core user journeys:

1. Clone a normal repository, edit with ordinary tools, inspect/commit/rebase, share via GitHub/GitLab (Git adapter).
2. Clone/fetch a native source tip without all historical blobs, then expand history/paths on demand; explicitly obtain a full offline repository/backup.
3. Make changes or compare source by immutable ID without a working copy; reject stale conditional writes and ref races.
4. Manage multiple independent or dependent changes; compose exact candidate source for review/test, optionally in one working directory.
5. Preserve captured unfinished work separately from authored history and recover after interruption or source checkout failure without overwriting a later editor's changes.

**Not owned by AVC:** an agent harness/orchestrator, editor CRDT, test/build runner, runtime/VM snapshots, environment setup, code review SaaS, deployment platform, or mandatory hosted forge. These use AVC's source APIs as clients; they need not contaminate source identities.

**Optimization target:** correctness and the end-to-end cost of useful tasks under actual CPU, SSD, network, memory and concurrency conditions, rather than optimizing an isolated hash or claiming universal superiority over Git/Scion/Jujutsu.

## 2. Logical model and one source authority

**Selected:** immutable **snapshot-root persistent Merkle source trees** with incremental path-copy updates, plus immutable authored revisions and distinct stable logical ChangeIds. This is not a full copy of a repository per revision. A changed path rewrites only touched directory-map nodes and parent links; unchanged blob/tree IDs are reused.

- Canonical **Blob**: exact bytes; no automatic text normalization.
- Canonical **Map**: directory bindings with file/executable, symlink target, directory, or unresolved conflict.
- Canonical **Conflict**: normalized signed exact-entry terms (plus/minus), with lossless opaque binary/type/delete alternatives and bounded composition. Merge driver results are exact source values, never marker text as authority.
- Canonical **Revision**: exact source root, ordered parent RevisionIds, authored metadata and stable logical ChangeId. Amend/rebase produce a new immutable RevisionId, not in-place mutation.
- **Private local view and operation log:** mutable source selections, work handles, ref generations, checkpoint roots and transaction receipts in a separate durable metadata authority.
- **Derived data:** changed paths, rename/copy heuristics, blame, path history, source search, rev graph accelerators, Git mappings and review/test attestations. All can be rebuilt or separately maintained; none is a second canonical source/history interpreter.

A complete source root uniquely describes one **typed source value**, including unresolved alternatives, independently of its history and physical compression. Two histories can share the same source root. Authored history has ordered parents; unknown foreign/promised ancestors are never fabricated as root revisions. A ChangeId identifies ongoing authored work, not a context-free patch or a permission to apply it to any arbitrary base.

Canonical v1 is precisely specified in [format.md](format.md): typed domain-separated BLAKE3-256 IDs, fixed-width little-endian deterministic payload encoding, path-compressed radix with 32-binding leaves, and flat signed conflict expressions. **Those choices are selected for implementation**, not indefinitely open architecture votes; fix a demonstrable flaw during unreleased qualification and version released changes.

## 3. Fundamental invariants

1. **Exact source:** every admitted complete native source root reconstructs the same exact typed namespace independent of origin, ancestry, OS or storage location.
2. **Single authority:** only canonical immutable source objects define source bytes/conflict values. A change log, foreign SHA, cached index or operation receipt never supersedes them.
3. **Honest durability:** success for a protected root, backup, checkpoint or remote publication implies its declared owning closure is verifiably durable in the stated domain. A partial client is not a full offline copy.
4. **No false absence:** a promised/unknown/unauthorized/missing object or ancestry is not an empty file/directory, a nonexisting path, or a valid root ancestor.
5. **No lost uncertainty:** meaningful unresolved source alternatives survive as typed values. If operation/resource limits prevent safe source construction, retain the exact inputs and return pending/refusal.
6. **Conditional effects:** shared ref updates and source edits have exact expected-version/value witnesses. No implicit force overwrite or last-writer-wins.
7. **No false filesystem ownership:** ordinary working directories can be written by external processes; source observation is not an atomic multi-file instant. Materialization is guarded and crash-recoverable, not assumed atomic.
8. **Offline-first, service-optional:** a full native repo/backup can read, edit and verify retained source without Git, daemon, filesystem mount or network.
9. **No unrequested execution/disclosure:** reading/importing/materializing source does not execute repository hooks, generators, agents, build steps or downloaded code; private checkpoints and evidence are not shared implicitly.
10. **Foreign honesty:** Git object identities cannot be converted into native BLAKE3 IDs without their bytes. An incomplete foreign Git view remains distinct from a verified native source root.

## 4. Selected storage architecture

**Selected local backend:** one transactional SQLite (WAL/FULL) metadata/root authority plus immutable, bounded staged payload segments; small objects may reside in SQLite. Logical hashes are computed from canonical uncompressed bytes. Physical segments can use raw or Zstandard, grouping and chunking independently of canonical IDs.

A durable write makes **object data and segment locations durable before a short SQLite root/pin CAS commit**; acknowledgement follows the completed transaction. Crash before metadata commit may leave unreachable staged/sealed bytes, never a published root with missing dependencies. Readers pin immutable segment generations; compaction changes object locations atomically and retires old segments only after readers/backup/GC protections release. GC traces exact owning roots and uses generation fencing. A full export follows a pinned owning closure and independently restores without a service. The detailed failure windows, SQLite authority records, reader leases and filesystem protocols are [in storage.md](storage.md).

A SQLite-only comparison backend remains a *qualification control*, not a second persistent production architecture. If the hybrid cannot meet equivalent durability at acceptable maintenance/performance cost, replace it coherently **before custody release** without changing canonical source objects.

## 5. Source edits, conflicts, history and parallel work

**Source edit API:** read/diff/apply/merge operate by immutable exact roots and typed path witnesses. A small update uses structural sharing, not the old prototype's FlatMap whole-tree rebuild. A native thin client with a trusted complete root and validated Merkle path/sibling proofs may compute a conditional new root without all sibling bytes; the full holder validates and durably owns all missing dependencies before publication.

**Merge:** versioned deterministic three-way value merging with explicit known base(s); independent content/mode changes may combine; binary, directory, symlink and deletion conflicts preserve typed alternatives. The canonical expression is **normalized signed multiset terms** to avoid uncontrolled nested same-path conflict values. Unknown merge base or transient resource failure yields a pending result, not a fabricated semantic conflict. A source result may be unresolved yet authored; conflict presentation is not source authority. Specific rules and bounded text driver are in [format.md](format.md).

**History editing:** stable ChangeIds through amend/rebase, exact predecessor/successor mapping for split/fold/divergence, and scoped transactional operation-log undo. Histories and immutable objects are retained only while protected by refs/pins/retention, not because audit metadata happens to mention an ID.

**Composed changes:** GitButler-like independent/dependent stacks are a higher-level **derived view**, not another canonical patch graph. A human can select stacks in one directory, assign hunks to specific changes, or perform optional `absorb` with unambiguous preview. Agents can use distinct writable directories, explicitly coordinated shared views, or checkout-free conditional source APIs. Filesystem/process isolation is different from source change ownership.

## 6. Native fetch and Git adoption

Native default resolves one exact selected current ref, fetches current working source for ordinary tools, and retrieves a bounded (initially 64 first-parent revisions) history horizon rather than all old file blobs. Sparse/no-checkout agents obtain requested objects and authenticated path proofs, batch missing data, and can deepen/fully hydrate. Full offline completeness is an **explicit verified promise**. Native refs publish with durable closure and per-ref conditional CAS, not global branch locking. [sync.md](sync.md) owns transport, availability states, security and consistency.

**Git is early adoption, not native backing.** For ordinary repos, import/verify all selected tip source bytes into a native root, retain a *foreign ancestry frontier* for missing old Git history, and export/push representable native commits against the exact Git remote/ref with a lease. A huge filtered Git repository may instead be opened as an **explicit foreign partial view** and support Git-relative edits/push without claiming native source IDs or offline complete custody. See [git.md](git.md); no hidden dual-write Git checkout by default.

## 7. Implementation shape and boundaries

The Rust implementation should have one owner for each concern, organized by **clear types and failure boundaries**, not one crate per conceptual noun:

- Canonical codec/ID and persistent source tree library.
- Pure conditional source editing, diff, merge, revision and conflict logic.
- Physical object backend + local SQLite root/operation authority, reader leases and GC.
- Filesystem capture/guarded materializer, private recovery and local workspace selection.
- Native object/proof/remote and separate foreign Git adapter.
- Thin CLI/TUI/agent bindings that call the same library rather than reimplementing semantics.

Start with a **thin vertical path**: exact bytes → persistent tree edit → protected durable root → commit/show/restore → independent backup/verify → Git import/export → native sparse client. No production scaffold merely for a future optional service. Keep APIs typed: `Stale`, `NeedObjects`, `Unauthorized`, `Corrupt`, `Conflict` and `Pending` are distinct. Use content hashes as integrity, not authorization; scope remote path proofs to avoid information leakage.

## 8. Design freeze and falsification

**The architecture is fully selected for implementation.** Remaining work is executable qualification, not a new general architecture contest:

- Independent codec vectors, insertion-order/property/fuzz tests and incremental radix edits.
- Typed signed-conflict algebra, real text merge, repeated rebase/revert and adversarial file/path/binary cases.
- Crash/power-failure and GC/read-lease/backup tests of the chosen physical store versus a SQLite-only control.
- Real small Git repo round trip; honest huge filtered foreign Git source; native selective clone and conditional agent edits.
- Measured cold/warm end-to-end tasks vs Git/Jujutsu and representative native workloads.

If one exact counterexample invalidates an invariant or a controlled complete-task comparison exposes an unacceptable design cost, revise the owner and **replace** the unreleased format/code coherently. No stable IDs or claims of safe irreplaceable-source custody before those checks. There is no need to wait for more general research to start implementation.

**Current repository state:** see [README](../README.md). The superseded experimental codec, its tests/fixtures and associated CI were intentionally retired from the active tree; old code/results are available at [the preserved pre-cleanup commit](https://github.com/nijaru/avc/tree/af66a63898f383b596e4828199dc625a80e28a72). The next local session should implement from the current specs, not resurrect that obsolete format.
