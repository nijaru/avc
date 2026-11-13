# AVC local storage, durability, recovery and GC

**Implementation contract, updated 2026-11-12.** Owns *physical storage and mutable local authority*, not canonical source encoding ([format](format.md)), network semantics ([sync](sync.md)) or CLI policy ([UX](ux.md)). This is a chosen design; durable admission rests on the documented SQLite WAL/FULL transaction contract, the same class of upstream durability contract Fossil's SQLite store relies on. AVC qualifies its own integration (§9) and does not certify arbitrary OS or hardware, so no immunity from power loss is promised.

## Current implementation boundary (November 2026)

`src/repository.rs`, `src/repository/history.rs`, `src/repository/rebase.rs`, `src/repository/git_bridge.rs`, `src/repository/transfer.rs` and `src/repository/references.rs` implement **selected source, authored history, tip amend, scoped undo, single-change rebase, independent named revision references, selected-history transfer and local Git translation** on **physical schema 12: one SQLite WAL/FULL database owns payloads and local metadata**. Every bounded canonical payload is an immutable `BLOB NOT NULL` row (typed ID, canonical length, payload); selection CAS and typed before/after receipts commit **in the same transaction** as new payloads. There is no raw-object directory, per-file installer, backend flag, trait or second native payload store. Database triggers make objects and receipts immutable. Nothing is deleted: no GC, compaction, location replacement or reader leases exist; future deletion needs its own lifecycle (§5).

Physical limits are explicit: 16 MiB per canonical payload, 256 MiB per owning closure, 100,000 objects. Operations buffer bounded closures and may hold multiple copies. The SQL engine's row-length limit is set to 16 MiB + 1024 bytes of record overhead on every open (and verified after setting), bounding BLOB materialization inside SQLite before Rust validation. Compression, segments/extents, streaming beyond these bounds, incremental persisted verification, recovery checkpoints and in-place materialization remain unimplemented or deferred (§5). Source loads/admissions revalidate complete source closures, not just changed paths. `commit`/`amend` validate selected source and direct-parent metadata; initial rebase loads/verifies the unique union of both input history graphs and separately bounds its resulting owning graph; `revision`/`log` read checked metadata without traversing every historical file. Explicit verification of a Revision root traverses its full bounded owning graph. Administration directories are assumed trusted against hostile concurrent replacement; project source and imported object streams are not trusted. Process termination around SQLite commit, concurrent CAS/dedup, corruption and independent-store round trips have passing tests on both November 6 control variants ([research §14](research.md#14-sqlite-only-storage-decision-and-equivalent-work-control-november-6-2026)); further fault cases extend with each new lifecycle, and OS/device power-loss behavior is outside the declared failure model and never promised (§9).

Initialization and every recognized reopen complete directory installation sync and a checked SQLite `FULL` WAL checkpoint before returning a write-capable handle. Receipt replay uses the same fence: a visible recovered commit is not enough to acknowledge durability. A busy or incomplete checkpoint returns `DurabilityPending`; it can reduce availability while another reader/writer holds a snapshot. SQLite alone owns database/WAL descriptors—**nothing outside SQLite opens and syncs `store.sqlite`**; opening and closing an unrelated descriptor for its database can release POSIX locks. Request preparation reads prior receipt and selection in one SQLite snapshot.

Every operation on a repository handle rejects a surviving transaction with `RecoveryRequired`. COMMIT failure does not imply rollback succeeded: if cleanup also fails, the handle must be dropped before reopening, never allowed to report its own uncommitted receipt. The library-owned `export_file` and `export_history_file` complete exclusive creation, checked platform file sync (including macOS F_FULLFSYNC), parent-directory sync and a final file/device flush; the generic writer export leaves publication to its caller.

Copying `store.sqlite` alone from an active or unchecked store is **not a safe backup**: the WAL may own committed transactions the main file does not contain. The supported transfers below carry either one exact source root or, with explicit `--history`, the selected source and optional authored head with all native owning ancestry. Neither includes named refs, local receipts, instance identity, workspace settings or derived Git maps; neither is a complete repository/workstation backup. Selected history carries retained foreign Git boundaries in `AVCHIS02`; native-only history retains unchanged `AVCHIS01` bytes. Source-only export omits authored history and origins. See [Git scope](git.md#available-local-round-trip).

`src/materialize.rs` and `restore` implement fresh-directory output only: preflight the complete source and resource/platform bounds, exclusively create the destination, use anchored Unix directory handles without following symlink parents, and sync output files/directories. Preserve exact file bytes, executable bits, symlinks and empty directories. Conflicts refuse before creation. An error can leave an incomplete new directory, retained for inspection; there is no automatic cleanup, resume journal, workspace registration or overwrite of an existing destination. This is not the in-place materialization/recovery protocol below.

The target protocols below describe future capabilities and lifecycles (restacking, broader capture, in-place overwrite, GC) and are implemented and checked only when those features are built, not guarantees inferred from this subset. The CLI requires explicit `--experimental` for repository mutations and does not implement workspace recovery, descendant restacking or native hosting; survival of power loss on arbitrary hardware is not promised.

## 1. Chosen engine and store ownership

- Rust library with one **SQLite WAL/FULL store** owning canonical payload rows, the selection and immutable receipts, short transactions and process-safe writer coordination; no mandatory daemon. No backend flag, provider trait or alternate physical store.
- Every supported bounded payload lives in one `objects` row. Stable object IDs are typed hashes of canonical payload bytes. **Canonical v1 IDs and the `AVCEXP01` object-only stream are independent of physical storage and unchanged by schema 8.** No Git packfiles in native identity.
- Physical compression, segments/extents and streaming beyond the 16 MiB payload bound are **deferred**, to be introduced only on measured need (for example bounded-RAM streaming of larger payloads, or demonstrated WAL/checkpoint amplification at sustained multi-GiB scale). They would be physical choices that cannot change canonical IDs. The November 6 equivalent-work control ([research §14](research.md#14-sqlite-only-storage-decision-and-equivalent-work-control-november-6-2026)) found no measured benefit for the hybrid at the supported bounds; segments are not part of the selected architecture.
- Initial repository layout (implementation may change cosmetic paths only before release):

```text
.avc/
  store.sqlite                # payloads and local metadata
  store.sqlite-wal            # SQLite-managed; may contain committed transactions
  store.sqlite-shm            # SQLite-managed coordination; companions may be absent
```

Repository format/schema version and instance ID belong to SQLite (`application_id`, `user_version` and the repository row), not a competing mutable manifest. **Physical schema 12** (`user_version = 12`) is current. Opening any other schema version or an unrecognized database is a clear refusal with **no automatic migration**; `init` refuses any existing `.avc`, including schema-11 portable-origin stores, schema-10 capture stores, schema-7 Git stores, schema-6 rebase stores, schema-5 amend/undo stores, schema-4 history-transfer stores, schema-3 authored-history stores, schema-2 SQLite-only stores, earlier schema-1 hybrid layouts or any other unsupported store. Paths under `.avc` are excluded from source capture even when users pass broad patterns. Never silently import, overwrite or delete a preexisting `.avc` layout. Avoid storing credentials in source or diagnostic dumps.

## 2. SQLite authority and local record model

This is the **minimum logical schema**; indexes and row normalization are implementation choices as long as constraints are preserved transactionally. Schema 12 implements:

| Record | Key and important fields | Protected effect |
| --- | --- | --- |
| `repository` | singleton row, 16-byte instance ID | Store identity |
| `objects` | typed ObjectId primary key (33 bytes), canonical length 0..=16 MiB, `payload BLOB NOT NULL` with `length(payload) = length` | Exact canonical source/revision bytes and dedup; immutable by trigger |
| `selection` | singleton row, source root, optional authored RevisionId, monotonic generation | One CAS generation for selected source and authored head |
| `operations` | global request key (1..=1024 bytes), selection-operation kind with before/after source/head/generation and kind-specific inputs, or `ref-set` with name, predecessor generation, new generation and target RevisionId | Immutable typed receipts; selection effects commit with payloads; ref rows own both reference state and acknowledgement. Capture includes its exact request scope (§6); history import includes a complete checksummed origin manifest (§8) |
| `capture_observations` | request-key foreign key, bounded observation witnesses and root/scope-bound checksum (§6) | Private retained evidence, admitted atomically with capture source/receipt |
| `git_origins` | exact anchor RevisionId, unique algorithm-tagged Git commit, tree and ordered foreign parents, original raw commit and bound BLAKE3 checksum | Retained foreign-boundary authority; immutable, committed with anchor objects/selection/receipt |
| `git_exports` | exact native RevisionId, Git algorithm and export policy version → Git commit | Immutable derived translation mapping, not a Git ref or source authority |

Initialization selects empty source without authoring a revision. Source edits advance generation while preserving the authored head. `commit` preserves source, generates a new Revision with the prior authored head as its sole parent (none initially), then advances head and generation. Explicit unchanged/empty-source commits are allowed. A new commit gets an OS-random 16-byte ChangeId and checked UTC nanoseconds, generated only after the transaction's receipt/CAS recheck. Author must be nonblank and at most 4 KiB; message may be empty and is at most 1 MiB. Identical retries match operation kind, expected generation and author/message, returning the original revision/ChangeId/time even after later changes. Cross-kind key reuse refuses.

`log` follows first parents newest-first, bounded to 100,000 records and 256 MiB of aggregate canonical metadata; it returns an error rather than a partial result on corruption or budget exhaustion. These limits do not promise full historical-file verification on metadata reads. `import-history` restores the declared source/head into an untouched initialized store (generation 0), publishing payloads, retained origins, both selections and its receipt atomically at generation 1. Its exact retry identity is operation kind, expected generation 0, selected source, optional head and the complete accepted origin declaration; an identical retry returns the original receipt even after later changes. A changed valid selection, changed origin declaration or cross-kind request-key reuse refuses; a new request on a used store returns `NotEmpty`. Original generations, instance identity and request keys are not imported. `amend` authors selected source while preserving the tip's ChangeId and exact ordered parent list, not parenting the superseded tip. It uses new explicit author/message metadata and a new timestamp; even an unchanged amend creates a distinct Revision. Init/unborn refuses. Both authoring kinds reconcile exact retries before changed invalid metadata is validated; kinds cannot reuse each other's keys. Predecessors remain retained locally.

`undo` names the exact receipt whose after-source/head **and generation** still match the current selection. It restores that receipt's before-source/head values with a new forward generation and receipt, never rewinding generation counters or modifying files/remotes. Its identity includes the target request key and expected current generation. Undo can itself be undone under the same rule; if any later operation advanced the selection, naming an older target refuses. Ref-scoped receipts cannot be undone through selection undo.

`rebase` requires an authored head, selected source equal to that head's source, and zero or one original parent. Its comparison source is exactly the original parent's source (empty Map for a parentless change), not an inferred common ancestor. It merges that base, the exact target source and the original source, then publishes the result with the target as sole parent. ChangeId, author, message, authored nanoseconds and timezone are preserved. Target ancestry already containing that ChangeId refuses; this initial scope does not restack descendants or resolve divergent versions of the same change. A no-op onto the existing parent retains the exact RevisionId but advances generation with a receipt. Identity is operation kind, target RevisionId and expected generation, reconciled before current-state checks; an exact retry returns its original receipt even after later changes. Undo treats that receipt like any other current source/head operation. Genuine conflicts remain typed source, while `UnresolvedBase`, missing objects and resource refusal publish nothing. Optional post-ack comparison diagnostics cannot turn a committed outcome into failure.

`import-git` admits a verified complete current Git tip into an untouched store, atomically selecting its deterministic native anchor, retaining exact original metadata/foreign parents and writing its typed receipt. It does not fetch old ancestry or snapshot refs; request identity is the exact tagged commit, not a directory. Retained origin checksums and the native ChangeId bind the record; export additionally verifies complete anchor/source binding and reconstructs the exact original current-source objects when absent at its target. An acknowledged import retry needs no surviving Git store. Undo restores selection values, not deletion of origin records. Amending/rebasing the anchor itself refuses; native children can be edited normally.

Git export verifies the bounded native owning graph, prepares all representable Git source/history and checks retained mappings and preflights every planned target object before any Git object write. Present objects must match planned bytes; malformed, corrupt or failed reads never mean missing. It preserves known exact foreign origins and translates parentless native revisions without such a boundary into Git roots. A known import anchor with missing/damaged origin metadata refuses; it cannot fall back to native-root export. Objects are installed in the separate Git store before a short SQLite transaction records derived mappings keyed by native revision, SHA-1 algorithm and policy 1, followed by the checked durability fence. Git and SQLite are not one transaction. No native selection/generation, Git ref/index/worktree or network state is changed. A late error may leave immutable unreferenced Git objects; deterministic retry reconciles the mapping. The canonical graph, planned Git objects and origin metadata each have aggregate bounds; buffering can hold multiple copies. [Git](git.md#available-local-round-trip) owns the metadata projection, supported subset and the whole-target physical-storage check needed for planned empty trees; that Git subprocess scan is not bounded by the planned closure’s size.

### Independent named revision references

A local name points to an already-stored NativeRevision and owns its own positive generation. Missing names expect generation 0 and start at 1; every accepted set advances once, including a same-target set under a new key. No ref mutation consults or advances the selected source/head generation, and authoring or rewriting never moves a name implicitly. Names and CLI behavior are specified in [UX](ux.md#local-named-revision-references).

There is **no mutable ref-head table or mirrored receipt**. Immutable `ref-set` operation rows own the transitions and current value is the latest generation for that name. Selection fields and their effect-specific metadata are NULL for ref rows; all ref fields are NULL for selection rows. Explicit presence checks enforce these disjoint shapes. The full unique `(ref_name, after_ref_generation)` index is the parent of a `(ref_name, before_ref_generation)` self-foreign-key. Creation has a NULL predecessor; updates require a positive predecessor and advance exactly once without integer overflow. These constraints establish a birth-at-1 chain without gaps or forks. The receipt's before-target comes from its immutable predecessor, never the current value. A required missing predecessor is damaged metadata, not creation.

A partial birth index has one entry per name. A list page ranges over that index and probes each name's latest generation in one SQL snapshot, rather than scanning all historical updates. Each page is consistent; separate pages are not a persistent snapshot. Metadata reads do not traverse target payloads. List limits bound returned rows (default 20, maximum 100,000), not lifetime names or updates.

A set uses `BEGIN IMMEDIATE`, reconciles the global request key first, checks that name's expected generation, validates stored Revision metadata and direct dependency presence, then inserts its single authoritative journal row and commits. It admits no payloads and never reads the full historical owning graph. Git import anchors are valid targets without consulting origin records. This acknowledgement is not a certificate of transitive target health. Identity is name, target RevisionId and expected ref generation; cross-kind key reuse refuses. An exact retry recovers its original receipt despite later source/ref moves or target payload damage, using the existing durability fence.

No deletion, rename, automatic branch advancement, checkout, ref undo, Git publication, network synchronization or reference transfer is implemented. Existing exports omit names and unrelated named tips. Nothing is physically removed, including superseded ref targets and receipts.

Later metadata remains future planning owned here; none of it exists today:

| Record | Key and important fields | Protected effect |
| --- | --- | --- |
| `workspace` | identity, path binding, selected revision/base, generation, active work association | Which source a directory is intended to show |
| `work` | stable local work ID, current base/candidate, protection state, owner workspace, generation | Resume unfinished work without a fake authored commit |
| `checkpoints` | capture ID, base, protected exact source root or captured entries, path/origin scope, exclusions, expiry or explicit pin | Private recovery with truthful retention |
| `operations` (extended) | kind, scope, expected generations, state beyond the retry receipt | Local undo and interrupted effects |
| `materializations` | op ID, exact before-image witnesses, planned path writes/removes, staged output and reconciliation state | Safe non-atomic multi-file checkout/restore |
| `remote_tracking` | remote/ref, foreign/native identity mapping, fetched frontier/availability, last known remote generation | Git/native sync is not an extra source authority |
| `object_promises` | authenticated native holder, target IDs/root, expiry/availability scope | Distinguish intentional missing objects from corruption |

Deletion/GC state and reader-lease records do not exist and are not required while nothing is ever removed; they arrive only with a measured reclamation need and a reviewed lifecycle (§5).

A transaction updates all *local mutable metadata* it owns under expected generation checks. SQLite is the single local ref/receipt authority; do not introduce independent authoritative JSON manifests or a second on-disk ref store.

**Concurrency:** multiple read transactions and immutable object readers operate concurrently. A local short `BEGIN IMMEDIATE` transaction serializes metadata writes (appropriate on a single device); there is no separate object-file installer or installer lock. The current source selection and authored head share one generation for one local selection. Named refs already own independent generations; future work/workspace selections must do the same; a head plus selected source that must advance together share one scoped transaction. Other independent scopes must not fail solely because a repository-wide generation advanced. Reuse this SQLite authority and operation journal rather than adding a parallel ref store or inferring work identity from content. A failed expected-generation CAS returns `Stale` and publishes no payload rows, selection or receipt; previously committed objects remain retained and in-memory candidates remain caller-owned. Never automatically retry an effectful operation against a newly observed revision without recomputing/representing the new plan.

## 3. Exact durable admission state machine

An operation involving a protected new root uses the following order. The invariant is **durable data before durable references**; in schema 12 both live in one SQLite transaction:

```text
Prepare (no acknowledgement)
  -> Capture exact input bytes; compute typed BLAKE3 IDs and canonical objects
  -> Validate new nodes/edges, expected base, and relevant owning closure
  -> Verify already-retained objects: typed hash/length and owned dependency rows
  -> BEGIN IMMEDIATE SQLite transaction
       recheck request receipt and expected generation
       for commit, generate ChangeId/time; for amend, preserve ChangeId/parents
       validate direct parent metadata and encode the new Revision
       insert new bounded payload rows (typed ID, length, BLOB NOT NULL)
       atomically CAS the source/head selection generation
       insert the immutable before/after receipt
     COMMIT with WAL synchronous=FULL
  -> Acknowledge success
```

Payload rows, the selection CAS and the receipt commit or roll back together. Concurrent same-content writers dedup into one immutable row; a recheck under the write transaction refuses a colliding row rather than accepting `INSERT OR IGNORE` results blindly.

Set and verify `journal_mode=WAL`, `synchronous=FULL`, `foreign_keys=ON`; on macOS also set and verify `fullfsync` and `checkpoint_fullfsync`. Propagate SQLite-reported I/O failures and AVC-owned sync errors. Bundled SQLite 3.53.2 can fall back from a failed macOS `F_FULLFSYNC` to `fsync`, even during commit; setting those pragmas does not prove that every device barrier succeeded. Optional close-time checkpoint errors may also follow an issued acknowledgement. Native fault tests therefore require coherent committed state for successful commands, not that every injected primitive failure becomes a command error. AVC-owned export barriers propagate failure without that fallback. Survival of OS/device power loss is not promised or qualified: acknowledged writes rely on the documented engine/OS sync contract—the same class of scoped durability contract a mainstream VCS declares (Git's default `core.fsync` policy explicitly permits losing recent work in an unclean shutdown). Payloads and metadata share one SQLite durability domain; initial directory installation and external exports still need their own checked syncs.

**Recovery/retry fence:** a complete WAL commit record may be visible after a failed final sync. Before acknowledging a recovered receipt, use SQLite's checked `PRAGMA main.wal_checkpoint(FULL)`: require no busy result and complete backfill, accepting the documented no-WAL counters only on a recognized WAL-configured handle. Release any local transaction before this fence. Do not substitute an empty transaction, a partial PASSIVE checkpoint or a raw `fsync` of the main database (which would also violate SQLite's descriptor ownership). A successful own FULL COMMIT already establishes its transaction's durability. A failed COMMIT/ROLLBACK leaving the connection non-autocommit retires that handle.

On read, check declared lengths and bounds before allocating, then verify the canonical typed BLAKE3 ID and the presence of direct owned dependency rows. A corrupt or incomplete row is not a normal cache miss for a retained locally complete root.

**Crash outcomes:**

| Crash point | Acceptable recovery |
| --- | --- |
| Before SQLite COMMIT | Prepared inserts roll back; no acknowledged root; nothing to clean up |
| During SQLite transaction | Transaction fully committed or rolled back per verified SQLite guarantees |
| After SQLite commit, before client response | Operation **may have succeeded**; retry by stable request key returns the exact original receipt; never execute an unknown-effect repeat |
| COMMIT and ROLLBACK both failing | Handle reports `RecoveryRequired`; drop and reopen before any acknowledgement |

Filesystem durability assumptions and unsupported network/overlay filesystems must be explicit; if they cannot be checked, refuse a **durable guarantee** rather than quietly weaken it.

## 4. Verification, closure and physical identity

**Full ownership closure** of a protected local **complete** revision includes its root tree, every map node, referenced Blob, Conflict terms and revision ancestors. A checkpoint may own a specific captured candidate source and its recorded base; an operation in progress owns exact before-images and staged outputs until resolution. Source metadata pointing to an unrelated input by ID is not automatically an owning retention edge; label non-owning audit commitments separately.

Reading any retained immutable object verifies its typed hash and canonical length and requires a row for every direct owned dependency; a missing dependency row is damaged storage, not a cache miss and not something admission may silently repair by re-inserting.

Object admission need not *re-hash or traverse* every unchanged subtree on every edit: immutable verified base roots and trusted cached node summaries can establish ownership of already retained subtrees. Newly admitted objects and all changed owning links must be validated. **Importing untrusted roots** requires full verification or a verified trusted authority with explicit signed/path commitments. Never use a borrowed verification flag from a different codec version/authority.

### Resource scope and repository incrementality

**Current behavior:** the 16 MiB payload, 256 MiB closure and 100,000-object bounds apply to buffered repository graph operations. A Revision's full owning graph includes every ancestor's source, so explicit full-history verification can exceed a closure limit even while each source snapshot fits. For example, eighteen distinct 15 MiB file versions already exceed 256 MiB of historical payload. That is a resource refusal, not proof of corruption or a limit on valid retained repository history.

**Target:** keep per-object validation, source-operation limits, metadata pages and history-transfer/traversal budgets separate. History traversal/transfer needs bounded working memory independent of total retained history; a complete manifest/result must never be silently truncated. Initial history-transfer support retains an explicit aggregate refusal limit, without making that limit a canonical validity or lifetime storage rule. This does not require segments, larger Blob support or removal of current bounds before the first small history/Git workflows.

Store-backed reads and reuse of verified unchanged objects are required for incremental repository edits; today's whole-source loads do not meet that goal. Establish verification provenance within the store/admission path, validate fetched bytes and changed edges, and retain explicit full verification and full untrusted-import checks. A caller-supplied flag is not proof of completeness. Measure the real command path, not only in-memory tree edits; this target does not silently relax current corruption checks.

A thin local repository has **incomplete custody** even if its selected root hash is authentic. Promised objects can be deleted from a disposable *cache* when redownloadable and policy allows; they cannot be deleted from a declared complete offline root. For `verify`, report `complete`, `partial-promised`, `missing-unexpected`, `corrupt`, or `denied`, not one misleading green result.

## 5. Deletion, GC and deferred physical work

No stored objects or receipts are deleted today: no GC, compaction, vacuum policy, location replacement or reader leases exist, and retained rows are immutable and never removed. Logical source removal, including explicit capture of missing bindings, retains the old source through its operation receipt. Concurrent readers need no lease while that holds.

Deletion/GC, compression, segments/extents and streaming beyond the bounded model are **deferred to measured need**, not part of the selected architecture. When such a need is demonstrated (reclaiming unreachable rows, bounded-RAM streaming past 16 MiB payloads, sustained large-blob WAL/checkpoint pressure), write and qualify the lifecycle here first: mark-and-sweep over exact owning edges from retained refs, pinned checkpoints, active unfinished work, materialization receipts/before-images and exports in flight; generation fencing against concurrent root changes; separation of unreachable payloads from promised-but-not-local remote objects; and refusal before destructive filesystem or row effects under quota exhaustion or missing payload. Do not use non-owning change lineage or mere mention in an audit report as an indefinite retention pin. Background cleanup may not silently weaken an acknowledged explicit pin.

**Checkpoint defaults (future policy):** explicitly pinned checkpoints stay until explicit discard; automatically generated *routine* protection may expire after **seven days**, but only when its exact expiry was disclosed at acknowledgement. Preserved unfinished work survives ordinary checkpoint expiry until successful verified resume or explicit discard. The user can inspect and extend protection; reclaiming an expired checkpoint doesn't delete bytes still protected by another root or export. Change history retained by refs stays until refs/pins are deliberately moved/removed and any future GC actually reclaims it.

## 6. Filesystem observation and materialization

**Implemented explicit capture:** `src/capture.rs` reads existing regular-file/symlink leaves on Linux and macOS. Explicit recursive mode also enumerates directory descendants, including new files and empty directories. `src/repository/capture.rs` publishes the resulting source, lexical absolute directory/raw path scope and inclusion/removal policy, private observation evidence and receipt atomically. The authored head and unselected source remain unchanged. A typed capture retry uses its original scope, mode and expected generation before any filesystem access. Scoped undo abandons the selected result without changing live files or authoring.

Leaf capture and ordinary `--recursive` capture refuse missing selected bindings. Explicit `--recursive --remove-missing` also removes old raw-name bindings absent from complete enumeration of an **existing parent directory**. A tracked missing file, symlink or directory may be requested directly: only its parent's names are enumerated, without capturing or reading unselected sibling payloads. Existing directory scopes still include their full descendants. A successful directory read to EOF, recorded sorted raw names and matching before/after directory witnesses are required; errors reading selected entries or their membership evidence abort the whole capture, never become deletions. Removed directory bindings include their selected descendants. This changes selected source only: no live files or immutable stored objects are deleted. Capture reports each removed binding before the user chooses whether to commit.

Untracked missing requested paths refuse rather than silently accepting typos. Missing filesystem parents still refuse, even with `--remove-missing`; select the missing tracked ancestor whose parent exists instead. Replacing selected source directories with files/symlinks also refuses rather than silently removing their children. Default tracked scans, ignore rules and hardlinked regular files remain unsupported. Recursive replacement of conflicted source refuses; resolve it explicitly first. Administration bindings are excluded before opening their files and their native source entries are preserved even when absent live. Pruning an absent ancestor that would discard protected administration bindings or unresolved conflicts refuses. Explicit administration scopes refuse. Counts, removed bindings and excluded, unobserved paths are diagnostics of the retained observation, not an atomic snapshot claim.

Scope is bounded to 1024 requested paths, 64 KiB of requested path bytes and a 4096-byte directory path. Empty relative path selects the root only in recursive mode. Expanded entries are bounded to 100,000, counting live observations, preserved Base-only exclusions and absent bindings; live exclusions already counted during enumeration are not counted twice. Canonical depth/path lengths and complete source payload/closure limits still apply. `operations.capture_scope` uses private `AVCCAP03` framing with an explicit inclusion/removal-mode byte, present only for capture. Private `AVCOBS03` evidence records complete raw name lists for scanned directories; an absent-binding record must name a missing raw child of one of those covered directories. Raw directory names, not case-insensitive pathname aliases, define native bindings. Mixed requests whose case/normalization aliases contradict covered raw names refuse rather than guessing a physical-name mapping. A membership-only parent record covers its names, not every child's bytes or source selection. Shared-parent absence requests reuse one guarded coverage record; detected membership changes refuse instead of silently refreshing evidence. `capture_observations` retains at most 16 MiB minus 4 KiB of versioned witness bytes per request, leaving SQLite row overhead room. An immutable row and a BLAKE3 checksum bind selected root, scope and witnesses. The deferred receipt foreign key and receipt-insertion trigger prevent orphan/missing observation publication. Explicit observation inspection verifies custody; receipt replay alone is not a storage-health certificate. These private records are not canonical source and are excluded from both export streams. Existing no-deletion retention applies.

CLI byte suppliers (`source put --file`, source/history `import --input`) also use the contained regular-file opener, not a plain independent open of an arbitrary pathname. Explicit supplier symlinks resolve to their target first, then anchored/no-follow parents, native SQLite inode exclusions, type and single-link checks apply before opening payloads. This prevents ordinary administration/hardlink aliases from independently opening SQLite-owned files without banning legitimate explicit exports inside `.avc/exports`. Bundle input must be an ordinary file; the library transfer API still accepts caller-supplied bounded streams. Byte suppliers do not promise stable contents or a simultaneous filesystem snapshot.

Source reads anchor directory descriptors, never traverse symlink parents, preserve symlink targets as raw data and project regular-file mode to the owner executable bit. Administration scopes/physical root aliases refuse before file reads; directory identity checks also guard aliases on case/normalization-insensitive filesystems. Before/open/after identity/stat witnesses detect observed races, including membership changes in enumerated directories; they are not a simultaneous snapshot proof. The broader checkpoint and materialization protocol below adds no extra guarantees to capture.

An ordinary directory is an **uncontrolled writer environment**. File watchers, stat caches, inode/file IDs, double scans and mtime are optimization/race indicators, **not proof** of a simultaneous filesystem snapshot.

**Capture:**

1. Resolve source root and requested tracked/new/ignored path scope, explicitly exclude AVC's administration directories; enumerate without following symlink escapes.
2. Open each relevant entry with containment-safe `openat`/`O_NOFOLLOW`-style operations (platform equivalent); record type, mode, observed bytes and lstat witness; stream bytes into durable protected storage.
3. Record scan coverage, unsupported types, read errors, concurrent modifications observed, excluded paths and any unobserved paths. A missing/failed read is not automatically a deletion. For authoring selected hunks, derive the candidate from **pinned captured bytes**, never from rereading live files after preview.
4. Do not claim every captured byte coexisted at one instant. Commit records the exact constructed logical tree and local observation receipt; a candidate can be valid even when its filesystem observation was not globally atomic.

**Materialization** is a separately staged, user-visible operation: `PREPARED` → `APPLYING` → `RECONCILING` → `COMPLETE` (or `NEEDS_RECOVERY`). First protect the departure work and exact before-image entries (including new obstacles, ignored paths at risk, and pending editor state **when observable**). Verify destination parent components/symlinks/case collisions; stage replacement file data on the destination filesystem, sync each staged file and affected parent directory before per-path atomic rename. Deletion/removal only occurs under exact precondition and explicit selected scope. Never traverse untrusted symlink parents.

**No multi-path atomic illusion:** another process can write between guards and rename. Use guarded operations and report any remaining race, and prefer an isolated materialization directory when control is uncertain. Do not declare the filesystem as clean/completed until all required planned writes and local view transitions have been reconciled. On crash, resume **only** when current path witnesses still match the plan's preconditions; otherwise preserve both protected old/new source and offer recovery into a fresh directory. Never blindly undo another editor's later writes.

A project switch does not copy the ignored environment, running processes, secrets, dependency cache, generated DB state, or unsaved editor buffers unless an independently authorized integration captures those things.

## 7. Operation journal and retry/undo

A local user operation is a durable transaction with:
`id`, `kind`, `workspace/work scope`, `before selection/generation`, `after selection/generation`, `dependent retained inputs`, `state`, and `idempotency_key`. For history-only operations, the before/after selection can be committed with the operation receipt in one SQLite transaction; immutable revisions never mutate. For filesystem effects, a materialization receipt and pinned before-images are committed **before** any file overwrite and reconciled afterward.

**Implemented metadata editing:** tip amend publishes a new Revision, the current singleton source/head selection and a typed before/after receipt atomically. It preserves logical ChangeId and ordered parents and retains predecessors and exact source inputs locally. Scoped undo names a committed receipt and restores its before-values only while its after-selection/generation still owns that scope. Both use the existing publication transaction and retry/recovery fence. A metadata-only rewrite is not a filesystem operation; no materialization journal is required when no files change. Single-change rebase uses that same transaction/receipt boundary, with its fully prepared source/revision outside the write lock. Independent ref/work scopes and descendant restacking remain future work.

Undo names an exact operation and checks the current affected selection/generation against the expected state. Restore its recorded source/head values in a **new forward transaction with a new monotonically increasing generation** and its own retry identity; never restore an old generation number or roll back the database. If that scope advanced, refuse rather than guessing; unrelated scopes are not an undo conflict. An undo of a commit/amend does not erase immutable objects, remote publication or arbitrary current filesystem edits. Returning to an unfinished effort and restoring its bytes is different from historical revision navigation.

Stable idempotency keys are per-request/scope, not universally reused across different plans. Repeated requests return the committed previous outcome or a typed still-pending state; do not repeat an ambiguous destructive operation after a crash or network loss.

## 8. Export, backup and disaster recovery

**Broader export target (beyond today's two streams):** a native self-contained export chooses exact roots/ref generations and an explicit retention/history scope, pins them before traversal, streams the **complete owning closure** into a versioned manifest + content-addressed object stream, and verifies every canonical ID and required relationship. Export is independent of the source storage backend and must be restorable with no Git or network service. Unknown promised/missing objects cause a refusal or a clearly labeled **partial transfer**, never a successful full backup.

An export manifest includes format/protocol version, chosen refs/roots, source repo instance ID, exact protected histories, foreign import metadata when selected, object count/checksums, and proof that EOF/manifest was completed. Do not include excluded private checkpoints or credentials by default. A backup includes mutable local refs/operation state only if explicitly requested; restoring shared Git identities or local filesystem locations may require a separate remap and does not grant publication permissions. An object-only export is not a complete workstation-environment snapshot.

Validate backup by independently restoring into an empty store and checking source bytes, revision ancestry and manifest integrity; then run representative local clone, read and diff without contacting the source. **Do not copy the main database alone from an active or unchecked store**: its WAL may own committed transactions, and an online file copy races writers. A database-level snapshot needs quiesced connections and a completed checked checkpoint, or SQLite's online backup protocol. Neither database-backup interface is implemented here; use the explicitly scoped streams below.

### Experimental selected-history stream

**Implemented:** `export --history` captures the selected source and optional authored Revision head in one SQL row read. It exports their **exact union owning closure**, including all ordered parents and historical source objects. Selected-but-uncommitted source is preserved separately from the head's source; an unborn head is valid. Immutable retained objects are never deleted, so later selection changes cannot invalidate that captured closure.

The stream includes canonical Revision IDs, parent ordering, ChangeIds and metadata byte-for-byte. Reachable retained Git origins accompany their anchors. It excludes named refs, local generations, instance identity, request keys/receipts (including undo), superseded revisions outside the selected owning graph, unrelated/private roots, workspace bindings and derived Git maps. CLI output discloses those exclusions. This is selected history, not all repository state or a complete foreign Git history backup. Native-only history uses the unchanged v1 framing:

```text
8 bytes "AVCHIS01"
33 bytes typed selected source-root reference
u8 authored_head_tag                  # exactly 0 or 1
if tag == 1:
    33 bytes typed Revision reference
u64le record_count                    # 1..100000
repeat sorted unique typed references (kind byte, then digest bytes):
    33 bytes typed ObjectId
    u64le canonical_payload_length    # <=16 MiB; unique sum <=256 MiB
    canonical payload
8 bytes "AVCHEND1"
32 bytes BLAKE3(all preceding bytes, including AVCHEND1)
EOF                                  # trailing bytes are invalid
```

When reachable retained Git origins are nonempty, use `AVCHIS02` with the same source/head/object section, followed by:

```text
u64le origin_count                    # 1..100000; authored head required
repeat sorted unique native Revision references:
    33 bytes typed zero-native-parent RevisionId
    21 bytes tagged Git OID           # algorithm 1, then 20 SHA-1 bytes
    u64le raw_commit_length           # <=16 MiB
    exact raw original commit
8 bytes "AVCHEND2"
32 bytes BLAKE3(all preceding bytes, including AVCHEND2)
EOF
```

Canonical payloads and raw origin commits share **one 256 MiB budget**. Every origin must be reachable through the selected authored head. Offline validation checks Git object framing with collision-detecting SHA-1, raw commit syntax and metadata, the complete deterministic native anchor ID, and the original tree hash reconstructed from native source. It preserves original empty-directory entries but does not expand native candidate policy. Foreign parent IDs remain opaque frontiers; their bodies are neither transferred nor fetched. These checks bind bytes and identities, not sender authenticity or every platform-specific Git admission rule.

The import receipt privately retains a complete `import_origins` declaration, including an empty declaration for v1. Its versioned `AVCGOR01` body contains a u32 count and sorted fixed claims `RevisionId[33] + GitOid[21] + BLAKE3(raw commit)[32]`, followed by a domain-separated BLAKE3 checksum. At most 100,000 claims occupy 8,600,044 bytes. A partial unique index permits only one fresh-store history import. The manifest owns retry identity and named origin custody; it is not another source authority. Missing or mismatched claimed origin rows refuse inspection/export and anchor rewrites rather than falling back to native roots. Exact retry compares the complete declaration without requiring healthy current raw-origin rows; damage to the manifest prevents that identity reconciliation.

The source reference must be a Map or root Conflict; a present head must be a Revision. Validate framing, bounds, canonical typed IDs, every contextual owning edge, exact union membership and completion before admission. A repaired outer checksum cannot authorize corrupt objects, missing ancestors or surplus records. Resource refusal is explicit, never a truncated successful backup.

Explicit `--experimental import --history` requires an untouched initialized store and publishes source/head, payloads, origins and a new typed receipt in one transaction (§2). The destination retains its fresh instance and advances its own generation from 0 to 1; original generations and requests gain no authority. Full input verification precedes publication, including on retry. Identical retry recovers its original receipt after later local changes. Old receipts and unrelated roots are not restored.

Export preflight verifies the entire selected graph before creating output. File exports use exclusive creation and checked file/parent sync; a later I/O failure can leave a partial file for inspection. Writer APIs leave publication to callers. Import/export modes are explicit: no automatic reinterpretation between this stream and `AVCEXP01`. The [roadmap](roadmap.md#portable-authored-history--implemented) owns sequencing and acceptance.

### Experimental object-only source stream

Default `export`/`import` (without `--history`) is **not the history stream or full backup manifest above**. It exports one exact source root (Map or root Conflict), its complete owning objects, and no local selections, request receipts, private discarded roots, repository instance, workspaces or authored refs. The selected source root cannot point at a Revision; the separate authored head is deliberately excluded. An explicit Revision export request refuses. Source-level restore is independent of the originating SQLite/files and does not require Git/network.

Its unreleased binary framing is:

```text
8 bytes "AVCEXP01"
33 bytes typed source-root reference
u64le record_count                    # 1..100000
repeat sorted unique typed references (kind byte, then digest bytes):
    33 bytes typed ObjectId
    u64le canonical_payload_length    # <=16 MiB; sum <=256 MiB
    canonical payload
8 bytes "AVCEND01"
32 bytes BLAKE3(all preceding bytes, including AVCEND01)
EOF                                  # trailing bytes are invalid
```

Verify the outer completion hash **and** each canonical typed ID/context/complete owning closure; a repaired outer checksum cannot authorize corrupt objects. Surplus non-owning records are invalid in this stream. Import requires an untouched initialized source store (generation zero), subject to exact same-request receipt replay. The output caller owns flush/sync/publication; CLI output uses exclusive creation and checked sync, never overwrite. A failed export can leave an incomplete output file, which must not import successfully. This is an experimental source transfer, not complete repository or workstation backup.

## 9. Engineering checks and scope

The fault suite injects deterministic failures at SQLite transaction boundaries, quota/disk-full conditions, sync calls, ref CAS updates, and export/restore and fresh-directory materialization steps, plus ordinary process crash and cancellation: scoped integration checks of AVC's own code paths, not certification of an OS or device. New lifecycles bring their fault cases when they are implemented (GC when deletion exists, in-place recovery when overwrite exists, workspace protection with capture); exhaustive OS/device power-loss qualification or a complete fault matrix is **not** a gate for authoring, history, Git interop or ordinary use, and power-loss immunity is never claimed. Security checks include oversized declared lengths and payloads (including hostile import streams), path traversal, symlink parent attacks, permission failures and crash immediately after acknowledgement.

**Recorded integration evidence:** common debug/release builds, cross-language Python vectors and the quota, rollback-failure, checkpoint-fence, POSIX-lock and process-crash suites passed on both variants of the November 6 control ([research §14](research.md#14-sqlite-only-storage-decision-and-equivalent-work-control-november-6-2026)). The recorded schema-2 real CLI native fault suite (`tests/io_faults.rs`) passed on macOS in debug and release: 240 source, 129 live/recovered-WAL receipt and 84 export cases, plus an independent shim ABI/scope/short-write probe. The source sweep replaces an already-acknowledged 12,000-byte file, verifies its exact retained predecessor after every failure and retries the replacement exactly once. Export checks include partial writes followed by EIO/ENOSPC or process exit. Each run discovers observed syscall boundaries rather than assuming a physical layout; vectored/copy writes, rename, deleted-open descriptors and raw syscalls are outside this harness. That suite and the full format/Clippy/debug/release/reference gates passed on both macOS and Linux in [schema-2 CI](https://github.com/nijaru/avc/actions/runs/42589777608). The schema-3 authoring slice passes those local gates, including the existing native fault suite; current Linux/macOS results are tracked by the [native workflow](https://github.com/nijaru/avc/actions/workflows/native.yml). These are scoped syscall errors and process termination, not lost OS/device caches or a hardware power-loss model. Remaining cases (disk-full at further write/sync points, GC/workspace-protection/backup lifecycles) arrive with the features they protect rather than gating authoring, history or Git interop. See the [roadmap](roadmap.md) for the current gate list.

**No hardware power-loss immunity is promised.** The format remains unreleased and experimental: commands stay `--experimental` and the README's independent-backups warning stands.
