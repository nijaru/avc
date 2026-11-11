# AVC local storage, durability, recovery and GC

**Implementation contract, updated 2026-11-09.** Owns *physical storage and mutable local authority*, not canonical source encoding ([format](format.md)), network semantics ([sync](sync.md)) or CLI policy ([UX](ux.md)). This is a chosen design; durable admission rests on the documented SQLite WAL/FULL transaction contract, the same class of upstream durability contract Fossil's SQLite store relies on. AVC qualifies its own integration (§9) and does not certify arbitrary OS or hardware, so no immunity from power loss is promised.

## Current implementation boundary (November 2026)

`src/repository.rs` and `src/repository/history.rs` implement **selected source and authored history** on **physical schema 3: one SQLite WAL/FULL database owns payloads and local metadata**. Every bounded canonical payload is an immutable `BLOB NOT NULL` row (typed ID, canonical length, payload); selection CAS and typed before/after receipts commit **in the same transaction** as new payloads. There is no raw-object directory, per-file installer, backend flag, trait or second store. Database triggers make objects and receipts immutable. Nothing is deleted: no GC, compaction, location replacement or reader leases exist; future deletion needs its own lifecycle (§5).

Physical limits are explicit: 16 MiB per canonical payload, 256 MiB per owning closure, 100,000 objects. Operations buffer bounded closures and may hold multiple copies. The SQL engine's row-length limit is set to 16 MiB + 1024 bytes of record overhead on every open (and verified after setting), bounding BLOB materialization inside SQLite before Rust validation. Compression, segments/extents, streaming beyond these bounds, incremental persisted verification, named refs, observation/checkpoints and in-place materialization remain unimplemented or deferred (§5). Source loads/admissions revalidate complete source closures, not just changed paths. `commit` validates selected source and checks retained parent metadata; `revision`/`log` read checked metadata without traversing every historical file. Explicit verification of a Revision root traverses its full bounded owning graph. Administration directories are assumed trusted against hostile concurrent replacement; project source and imported object streams are not trusted. Process termination around SQLite commit, concurrent CAS/dedup, corruption and independent-store round trips have passing tests on both November 6 control variants ([research §14](research.md#14-sqlite-only-storage-decision-and-equivalent-work-control-november-6-2026)); further fault cases extend with each new lifecycle, and OS/device power-loss behavior is outside the declared failure model and never promised (§9).

Initialization and every recognized reopen complete directory installation sync and a checked SQLite `FULL` WAL checkpoint before returning a write-capable handle. Receipt replay uses the same fence: a visible recovered commit is not enough to acknowledge durability. A busy or incomplete checkpoint returns `DurabilityPending`; it can reduce availability while another reader/writer holds a snapshot. SQLite alone owns database/WAL descriptors—**nothing outside SQLite opens and syncs `store.sqlite`**; opening and closing an unrelated descriptor for its database can release POSIX locks. Request preparation reads prior receipt and selection in one SQLite snapshot.

Every operation on a repository handle rejects a surviving transaction with `RecoveryRequired`. COMMIT failure does not imply rollback succeeded: if cleanup also fails, the handle must be dropped before reopening, never allowed to report its own uncommitted receipt. The library-owned `export_file` completes exclusive creation, checked platform file sync (including macOS F_FULLFSYNC), parent-directory sync and a final file/device flush; the generic writer export leaves publication to its caller.

Copying `store.sqlite` alone from an active or unchecked store is **not a safe backup**: the WAL may own committed transactions the main file does not contain. The supported transfer is the object-only export below, which carries one exact source root and never authored revisions, receipts or local state. It is not a backup of authored history.

`src/materialize.rs` and `restore` implement fresh-directory output only: preflight the complete source and resource/platform bounds, exclusively create the destination, use anchored Unix directory handles without following symlink parents, and sync output files/directories. Preserve exact file bytes, executable bits, symlinks and empty directories. Conflicts refuse before creation. An error can leave an incomplete new directory, retained for inspection; there is no automatic cleanup, resume journal, workspace registration or overwrite of an existing destination. This is not the in-place materialization/recovery protocol below.

The target protocols below describe future capabilities and lifecycles (history transfer/editing, capture, in-place overwrite, GC) and are implemented and checked only when those features are built, not guarantees inferred from this subset. The CLI requires explicit `--experimental` for repository mutations and does not implement workspace capture, history rewriting or native hosting; survival of power loss on arbitrary hardware is not promised.

## 1. Chosen engine and store ownership

- Rust library with one **SQLite WAL/FULL store** owning canonical payload rows, the selection and immutable receipts, short transactions and process-safe writer coordination; no mandatory daemon. No backend flag, provider trait or alternate physical store.
- Every supported bounded payload lives in one `objects` row. Stable object IDs are typed hashes of canonical payload bytes. **Canonical v1 IDs and the `AVCEXP01` object-only stream are independent of physical storage and unchanged by schema 3.** No Git packfiles in native identity.
- Physical compression, segments/extents and streaming beyond the 16 MiB payload bound are **deferred**, to be introduced only on measured need (for example bounded-RAM streaming of larger payloads, or demonstrated WAL/checkpoint amplification at sustained multi-GiB scale). They would be physical choices that cannot change canonical IDs. The November 6 equivalent-work control ([research §14](research.md#14-sqlite-only-storage-decision-and-equivalent-work-control-november-6-2026)) found no measured benefit for the hybrid at the supported bounds; segments are not part of the selected architecture.
- Initial repository layout (implementation may change cosmetic paths only before release):

```text
.avc/
  store.sqlite                # payloads and local metadata
  store.sqlite-wal            # SQLite-managed; may contain committed transactions
  store.sqlite-shm            # SQLite-managed coordination; companions may be absent
```

Repository format/schema version and instance ID belong to SQLite (`application_id`, `user_version` and the repository row), not a competing mutable manifest. **Physical schema 3** (`user_version = 3`) is current. Opening any other schema version or an unrecognized database is a clear refusal with **no automatic migration**; `init` refuses any existing `.avc`, including schema-2 SQLite-only stores, earlier schema-1 hybrid layouts or any other unsupported store. Paths under `.avc` are excluded from source capture even when users pass broad patterns. Never silently import, overwrite or delete a preexisting `.avc` layout. Avoid storing credentials in source or diagnostic dumps.

## 2. SQLite authority and local record model

This is the **minimum logical schema**; indexes and row normalization are implementation choices as long as constraints are preserved transactionally. Schema 3 implements:

| Record | Key and important fields | Protected effect |
| --- | --- | --- |
| `repository` | singleton row, 16-byte instance ID | Store identity |
| `objects` | typed ObjectId primary key (33 bytes), canonical length 0..=16 MiB, `payload BLOB NOT NULL` with `length(payload) = length` | Exact canonical source/revision bytes and dedup; immutable by trigger |
| `selection` | singleton row, source root, optional authored RevisionId, monotonic generation | One CAS generation for selected source and authored head |
| `operations` | request key (1..=1024 bytes), `source`/`commit` kind, before/after roots, authored heads and consecutive generations | Immutable typed retry receipts; source/head effects in one transaction |

Initialization selects empty source without authoring a revision. Source edits advance generation while preserving the authored head. `commit` preserves source, generates a new Revision with the prior authored head as its sole parent (none initially), then advances head and generation. Explicit unchanged/empty-source commits are allowed. A new commit gets an OS-random 16-byte ChangeId and checked UTC nanoseconds, generated only after the transaction's receipt/CAS recheck. Author must be nonblank and at most 4 KiB; message may be empty and is at most 1 MiB. Identical retries match operation kind, expected generation and author/message, returning the original revision/ChangeId/time even after later changes. Cross-kind key reuse refuses.

`log` follows first parents newest-first, bounded to 100,000 records and 256 MiB of aggregate canonical metadata; it returns an error rather than a partial result on corruption or budget exhaustion. These limits do not promise full historical-file verification on metadata reads. Named refs, amend/rebase and history transfer are not implemented.

Later metadata remains future planning owned here; none of it exists today:

| Record | Key and important fields | Protected effect |
| --- | --- | --- |
| `refs` | scoped ref name, target RevisionId, monotonic generation, retention policy | Atomic local published/draft/work selections |
| `workspace` | identity, path binding, selected revision/base, generation, active work association | Which source a directory is intended to show |
| `work` | stable local work ID, current base/candidate, protection state, owner workspace, generation | Resume unfinished work without a fake authored commit |
| `checkpoints` | capture ID, base, protected exact source root or captured entries, path/origin scope, exclusions, expiry or explicit pin | Private recovery with truthful retention |
| `operations` (extended) | kind, scope, expected generations, state beyond the retry receipt | Local undo and interrupted effects |
| `materializations` | op ID, exact before-image witnesses, planned path writes/removes, staged output and reconciliation state | Safe non-atomic multi-file checkout/restore |
| `remote_tracking` | remote/ref, foreign/native identity mapping, fetched frontier/availability, last known remote generation | Git/native sync is not an extra source authority |
| `object_promises` | authenticated native holder, target IDs/root, expiry/availability scope | Distinguish intentional missing objects from corruption |

Deletion/GC state and reader-lease records do not exist and are not required while nothing is ever removed; they arrive only with a measured reclamation need and a reviewed lifecycle (§5).

A transaction updates all *local mutable metadata* it owns under expected generation checks. SQLite is the single local ref/receipt authority; do not introduce independent authoritative JSON manifests or a second on-disk ref store.

**Concurrency:** multiple read transactions and immutable object readers operate concurrently. A local short `BEGIN IMMEDIATE` transaction serializes metadata writes (appropriate on a single device); there is no separate object-file installer or installer lock. The current source selection and authored head share one generation for one local selection. When adding named refs or independent work/workspaces, each ref/work selection owns its generation; a head plus selected source that must advance together share one scoped transaction. Other independent scopes must not fail solely because a repository-wide generation advanced. Reuse this SQLite authority and operation journal rather than adding a parallel ref store or inferring work identity from content. A failed expected-generation CAS returns `Stale` and publishes no payload rows, selection or receipt; previously committed objects remain retained and in-memory candidates remain caller-owned. Never automatically retry an effectful operation against a newly observed revision without recomputing/representing the new plan.

## 3. Exact durable admission state machine

An operation involving a protected new root uses the following order. The invariant is **durable data before durable references**; in schema 3 both live in one SQLite transaction:

```text
Prepare (no acknowledgement)
  -> Capture exact input bytes; compute typed BLAKE3 IDs and canonical objects
  -> Validate new nodes/edges, expected base, and relevant owning closure
  -> Verify already-retained objects: typed hash/length and owned dependency rows
  -> BEGIN IMMEDIATE SQLite transaction
       recheck request receipt and expected generation
       for authoring, generate ChangeId/time and encode the new Revision
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

**Target:** keep per-object validation, source-operation limits, metadata pages and history-transfer/traversal budgets separate. History traversal/transfer needs bounded working memory independent of total retained history; a complete manifest/result must never be silently truncated. Initial history-transfer support may retain an explicit aggregate refusal limit, without making that limit a canonical validity or lifetime storage rule. This does not require segments, larger Blob support or removal of current bounds before the first small history/Git workflows.

Store-backed reads and reuse of verified unchanged objects are required for incremental repository edits; today's whole-source loads do not meet that goal. Establish verification provenance within the store/admission path, validate fetched bytes and changed edges, and retain explicit full verification and full untrusted-import checks. A caller-supplied flag is not proof of completeness. Measure the real command path, not only in-memory tree edits; this target does not silently relax current corruption checks.

A thin local repository has **incomplete custody** even if its selected root hash is authentic. Promised objects can be deleted from a disposable *cache* when redownloadable and policy allows; they cannot be deleted from a declared complete offline root. For `verify`, report `complete`, `partial-promised`, `missing-unexpected`, `corrupt`, or `denied`, not one misleading green result.

## 5. Deletion, GC and deferred physical work

Nothing is deleted today: no GC, compaction, vacuum policy, location replacement or reader leases exist, and retained rows are immutable and never removed. Concurrent readers need no lease while that holds.

Deletion/GC, compression, segments/extents and streaming beyond the bounded model are **deferred to measured need**, not part of the selected architecture. When such a need is demonstrated (reclaiming unreachable rows, bounded-RAM streaming past 16 MiB payloads, sustained large-blob WAL/checkpoint pressure), write and qualify the lifecycle here first: mark-and-sweep over exact owning edges from retained refs, pinned checkpoints, active unfinished work, materialization receipts/before-images and exports in flight; generation fencing against concurrent root changes; separation of unreachable payloads from promised-but-not-local remote objects; and refusal before destructive filesystem or row effects under quota exhaustion or missing payload. Do not use non-owning change lineage or mere mention in an audit report as an indefinite retention pin. Background cleanup may not silently weaken an acknowledged explicit pin.

**Checkpoint defaults (future policy):** explicitly pinned checkpoints stay until explicit discard; automatically generated *routine* protection may expire after **seven days**, but only when its exact expiry was disclosed at acknowledgement. Preserved unfinished work survives ordinary checkpoint expiry until successful verified resume or explicit discard. The user can inspect and extend protection; reclaiming an expired checkpoint doesn't delete bytes still protected by another root or export. Change history retained by refs stays until refs/pins are deliberately moved/removed and any future GC actually reclaims it.

## 6. Filesystem observation and materialization

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

**Initial history-editing target:** publish rewritten Revisions, the scoped source/head selection and a typed before/after operation receipt atomically. Preserve logical ChangeId and retain predecessor revisions and exact source inputs for the declared undo scope. A metadata-only rewrite is not a filesystem operation; no materialization journal is required when no files change.

Undo names an exact operation and checks the current affected selection/generation against the expected state. Restore its recorded source/head values in a **new forward transaction with a new monotonically increasing generation** and its own retry identity; never restore an old generation number or roll back the database. If that scope advanced, refuse rather than guessing; unrelated scopes are not an undo conflict. An undo of a commit/amend does not erase immutable objects, remote publication or arbitrary current filesystem edits. Returning to an unfinished effort and restoring its bytes is different from historical revision navigation.

Stable idempotency keys are per-request/scope, not universally reused across different plans. Repeated requests return the committed previous outcome or a typed still-pending state; do not repeat an ambiguous destructive operation after a crash or network loss.

## 8. Export, backup and disaster recovery

A **native self-contained export** chooses exact roots/ref generations and an explicit retention/history scope, pins them before traversal, streams the **complete owning closure** into a versioned manifest + content-addressed object stream, and verifies every canonical ID and required relationship. Export is independent of the source storage backend and must be restorable with no Git or network service. Unknown promised/missing objects cause a refusal or a clearly labeled **partial transfer**, never a successful full backup.

An export manifest includes format/protocol version, chosen refs/roots, source repo instance ID, exact protected histories, foreign import metadata when selected, object count/checksums, and proof that EOF/manifest was completed. Do not include excluded private checkpoints or credentials by default. A backup includes mutable local refs/operation state only if explicitly requested; restoring shared Git identities or local filesystem locations may require a separate remap and does not grant publication permissions. An object-only export is not a complete workstation-environment snapshot.

Validate backup by independently restoring into an empty store and checking source bytes, revision ancestry and manifest integrity; then run representative local clone, read and diff without contacting the source. **Do not copy the main database alone from an active or unchecked store**: its WAL may own committed transactions, and an online file copy races writers. A database-level snapshot needs quiesced connections and a completed checked checkpoint, or SQLite's online backup protocol. Neither database-backup interface is implemented here; the object-only export below remains the supported source transfer.

### Next slice: selected-history transfer

This is **planned, not implemented**. Export an explicit authored head/revision set and its complete owning source/ancestor closure. Record the selected source separately when it differs from the head's source; do not discard selected-but-uncommitted bytes while describing a full local selection backup. The manifest declares exactly which selection/ref values and foreign-origin records it includes. Exclude request receipts, unrelated/private roots, credentials and workstation bindings unless separately selected; no promise of complete operation-log or workstation backup is implied.

Restore into a fresh store, verify canonical IDs, ordered parents, ChangeIds, source bytes and completion integrity, then publish the declared selection with a fresh local generation/instance. Imported source/head values do not authorize replay of the original store's generations or request keys. Preserve immutable object IDs byte-for-byte; missing dependencies or corrupt/incomplete streams must not publish a restored selection. If Git-origin history is selected, carry the origin/frontier mapping needed for the declared Git round-trip scope or explicitly report that it was excluded.

Define versioned history framing when implementing this slice; do not silently expand the source-only `AVCEXP01` contract. Keep source transfer and selected-history backup distinguishable in CLI scope and output. The [roadmap](roadmap.md#portable-authored-history--next) owns sequencing and acceptance; the final wire grammar will be maintained here.

### Experimental object-only source stream

The current `export`/`import` interface is **not the full backup manifest above**. It exports one exact source root (Map or root Conflict), its complete owning objects, and no local selections, request receipts, private discarded roots, repository instance, workspaces or authored refs. The selected source root cannot point at a Revision; the separate authored head is deliberately excluded. An explicit Revision export request refuses. Source-level restore is independent of the originating SQLite/files and does not require Git/network.

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
