# AVC local storage, durability, recovery and GC

**Implementation contract, 2026-11-04.** Owns *physical storage and mutable local authority*, not canonical source encoding ([format](format.md)), network semantics ([sync](sync.md)) or CLI policy ([UX](ux.md)). This is a chosen design, **not proven crash-safe until implementation/fault injection**.

## Current implementation boundary (November 5, 2026)

`src/repository.rs` implements an **experimental source-root-only subset**: SQLite WAL/FULL selection and immutable retry receipts, inline payloads through 1 KiB, raw immutable files above that threshold, complete-closure validation, conditional admission, reopening and object-only export/import. Receipts retain both source selections; no objects or receipts are deleted. There are no reader-expiry/GC races yet because no GC, compaction or location replacement API exists. This is not a substitute for qualifying those future lifecycles.

Physical limits are explicit: 16 MiB per canonical payload, 256 MiB per owning closure, 100,000 objects. Operations buffer bounded closures and may hold multiple copies. Compression, segment packing/extents, streaming large files, incremental persisted verification, authored refs, observation/checkpoints and in-place materialization remain unimplemented. Current loads/admissions revalidate complete closures, not just changed paths. Administration directories are assumed trusted against hostile concurrent replacement; project source and imported object streams are not trusted. Process termination around SQLite commit, concurrent CAS/dedup, corruption and independent-store round trips are tested; the complete fault matrix and OS/device power-loss behavior are **not qualified**.

Initialization and every recognized reopen complete directory installation sync and a checked SQLite `FULL` WAL checkpoint before returning a write-capable handle. Receipt replay uses the same fence: a visible recovered commit is not enough to acknowledge durability. A busy or incomplete checkpoint returns `DurabilityPending`; it can reduce availability while another reader/writer holds a snapshot. SQLite alone owns database/WAL descriptors—opening and closing an unrelated descriptor for its database can release POSIX locks. Request preparation still reads prior receipt and selection in one SQLite snapshot.

Every operation on a repository handle rejects a surviving transaction with `RecoveryRequired`. COMMIT failure does not imply rollback succeeded: if cleanup also fails, the handle must be dropped before reopening, never allowed to report its own uncommitted receipt. The library-owned `export_file` completes exclusive creation, checked platform file sync (including macOS F_FULLFSYNC), parent-directory sync and a final file/device flush; the generic writer export leaves publication to its caller.

`src/materialize.rs` and `restore` implement fresh-directory output only: preflight the complete source and resource/platform bounds, exclusively create the destination, use anchored Unix directory handles without following symlink parents, and sync output files/directories. Preserve exact file bytes, executable bits, symlinks and empty directories. Conflicts refuse before creation. An error can leave an incomplete new directory, retained for inspection; there is no automatic cleanup, resume journal, workspace registration or overwrite of an existing destination. This is not the in-place materialization/recovery protocol below.

The target protocols below remain release requirements, not guarantees inferred from this subset. The CLI requires explicit `--experimental` for repository mutations and makes no important-source custody claim.

## 1. Chosen engine and directory ownership

- Rust library with one **SQLite WAL/FULL metadata authority**, short transactions and process-safe writer coordination; no mandatory daemon.
- Immutable physically compressed **object segment files** for large data, optional small objects stored inline in SQLite. This is a storage implementation choice, **not a canonical format restriction**. No Git packfiles in native identity.
- Stable object IDs are hashes of canonical *uncompressed* payloads. Physical coding can use raw or Zstandard; the initial policy uses raw below 1 KiB, Zstd level 1 otherwise only when compressed output saves meaningful bytes. A future measured policy can change this without changing source IDs.
- Segments are immutable, addressed through SQLite object-location rows. A physical segment can contain many small objects; large Blob data can be streamed into multiple **physical extents** without changing one logical BlobId. Each extent carries length and checksum, and the ordered index reconstructs exact logical payload bytes. A partial extent/range read is not independently authenticated by the whole-Blob BLAKE3 ID until the complete Blob has been reconstructed and hashed, unless separately checked via an authorized trusted manifest; do not label it fully verified prematurely.
- Physical target: approximately 64 MiB of compressed payload per sealed segment with bounded memory buffering, but never require a whole large file or whole segment in RAM. Targets and compression levels are **tunable storage policy**, not canonical wire bytes.
- Initial repository layout (implementation may change cosmetic paths only before release):

```text
.avc/
  store.sqlite                # SQLite authority; WAL/SHM files managed by SQLite
  objects/
    staged/                   # unacknowledged physical segments
    sealed/                   # immutable, durably installed segments
  tmp/                        # recovery and materialization staging, explicitly tracked
  lock                        # local segment installer/GC coordination, not source identity
```

Repository format/schema version and instance ID belong to SQLite (`application_id`, `user_version` and the repository row), not a competing mutable manifest. Paths under `.avc` are excluded from source capture even when users pass broad patterns. Never silently import, overwrite or delete a preexisting old `.avc` layout. `init` refuses unknown/legacy content unless an explicit, separately qualified migration or fresh path is chosen. Avoid storing credentials in source or diagnostic dumps.

## 2. SQLite authority and local record model

This is the **minimum logical schema**; indexes and row normalization are implementation choices as long as constraints are preserved transactionally.

| Record | Key and important fields | Protected effect |
| --- | --- | --- |
| `objects` | typed ObjectId, canonical length, location (inline or segment/extents), validated codec/version, verification state | Exact physical source bytes and dedup |
| `segments` | segment ID, immutable path, length/checksum, state, generation | Which physical payloads may be read/retired |
| `refs` | scoped ref name, target RevisionId, monotonic generation, retention policy | Atomic local published/draft/work selections |
| `workspace` | identity, path binding, selected revision/base, generation, active work association | Which source a directory is intended to show |
| `work` | stable local work ID, current base/candidate, protection state, owner workspace, generation | Resume unfinished work without a fake authored commit |
| `checkpoints` | capture ID, base, protected exact source root or captured entries, path/origin scope, exclusions, expiry or explicit pin | Private recovery with truthful retention |
| `operations` | op ID, kind, scope, expected generations, before/after metadata, state, idempotency key | Local undo and interrupted effects |
| `materializations` | op ID, exact before-image witnesses, planned path writes/removes, staged output and reconciliation state | Safe non-atomic multi-file checkout/restore |
| `reader_leases` | unique session, protected segments/generation, liveness heartbeat or lock proof | Prevent deletion while readers use immutable files |
| `remote_tracking` | remote/ref, foreign/native identity mapping, fetched frontier/availability, last known remote generation | Git/native sync is not an extra source authority |
| `object_promises` | authenticated native holder, target IDs/root, expiry/availability scope | Distinguish intentional missing objects from corruption |
| `gc_state` | generation, pending retired segments and root-sweep cursor | Incremental safe reclamation |

A transaction updates all *local mutable metadata* it owns under expected generation checks. SQLite is the single local ref/receipt authority; do not introduce independent authoritative JSON manifests or a second on-disk ref store.

**Concurrency:** multiple read transactions and immutable object readers operate concurrently. A local short `BEGIN IMMEDIATE` transaction serializes metadata writes (appropriate on a single device); logical independent workspaces still advance via per-workspace/ref generation checks, not a global mutable `HEAD` inferred from content. A failed expected-generation CAS returns `Stale` with no ref change and retains any prepared objects under their stated temporary protection. Never automatically retry an effectful operation against a newly observed revision without recomputing/representing the new plan.

## 3. Exact durable admission state machine

An operation involving a protected new root uses the following order. The invariant is **durable data before durable references**:

```text
Prepare (no acknowledgement)
  -> Capture exact input bytes; compute typed BLAKE3 IDs and canonical objects
  -> Validate new nodes/edges, expected base, and relevant owning closure
  -> Write bounded payload extents to staged segments; verify their checksums
  -> fsync each new segment; install immutable final segment names
  -> fsync the sealed/ directory (and new parent directory when created)
  -> BEGIN IMMEDIATE SQLite transaction
       install object locations and sealed segment rows
       atomically CAS local expected ref/workspace generations
       create retained root/pin and idempotent operation receipt
       mark applicable old roots for later GC
     COMMIT with synchronous=FULL
  -> Acknowledge success
```

Set and verify `journal_mode=WAL`, `synchronous=FULL`, `foreign_keys=ON`; use platform-appropriate sync calls and fail when the filesystem/VFS cannot honor declared durability. On macOS evaluate `fullfsync`/F_FULLFSYNC availability; on Linux evaluate file/parent-directory fsync behavior. SQLite protects the metadata transaction but **does not automatically make separate segment files durable**. This ordering is part of the API contract.

**Recovery/retry fence:** a complete WAL commit record may be visible after a failed final sync. Before acknowledging a recovered receipt, use SQLite's checked `PRAGMA main.wal_checkpoint(FULL)`: require no busy result and complete backfill, accepting the documented no-WAL counters only on a recognized WAL-configured handle. Release any local transaction before this fence. Do not substitute an empty transaction, a partial PASSIVE checkpoint or a raw `fsync` of the main database. A successful own FULL COMMIT already establishes its transaction's durability. A failed COMMIT/ROLLBACK leaving the connection non-autocommit retires that handle.

Object hash input is streaming over canonical uncompressed bytes. The physical payload header/index includes codec ID, encoded and decoded length, checksum and bounded count; on read, check lengths and frame bounds before allocating, verify physical checksums, then canonical BLAKE3 ID. A corrupt segment is not a normal cache miss for a retained locally complete root.

**Crash outcomes:**

| Crash point | Acceptable recovery |
| --- | --- |
| Before seal+directory sync | Staged/truncated objects untrusted; no acknowledged root; cleanup only after checking pins and active sessions |
| After physical seal, before SQLite commit | Extra unreachable sealed object bytes can be reclaimed later; refs unchanged |
| During SQLite transaction | Transaction fully committed or rolled back per verified SQLite guarantees |
| After SQLite commit, before client response | Operation **may have succeeded**; retry by stable idempotency key to obtain exact result; never execute an unknown-effect repeat |
| During compaction or segment retirement | Either old complete location or new complete location remains readable through active leases; no acknowledged root loses closure |

Allow concurrent staging, but serialize sealing/location installation/retirement with a short process-safe lock. Do not rely solely on PID numbers for stale-lock/lease detection. Filesystem durability assumptions and unsupported network/overlay filesystems must be explicit; if they cannot be checked, refuse a **durable guarantee** rather than quietly weaken it.

## 4. Verification, closure and physical identity

**Full ownership closure** of a protected local **complete** revision includes its root tree, every map node, referenced Blob, Conflict terms and revision ancestors. A checkpoint may own a specific captured candidate source and its recorded base; an operation in progress owns exact before-images and staged outputs until resolution. Source metadata pointing to an unrelated input by ID is not automatically an owning retention edge; label non-owning audit commitments separately.

Object admission need not *re-hash or traverse* every unchanged subtree on every edit: immutable verified base roots and trusted cached node summaries can establish ownership of already retained subtrees. Newly admitted objects and all changed owning links must be validated. **Importing untrusted roots** requires full verification or a verified trusted authority with explicit signed/path commitments. Never use a borrowed verification flag from a different codec version/authority.

A thin local repository has **incomplete custody** even if its selected root hash is authentic. Promised objects can be deleted from a disposable *cache* when redownloadable and policy allows; they cannot be deleted from a declared complete offline root. For `verify`, report `complete`, `partial-promised`, `missing-unexpected`, `corrupt`, or `denied`, not one misleading green result.

## 5. Reader isolation, compaction and GC

- A reader obtains an immutable snapshot of logical object locations/ref generations and pins the physical segments it may access, using an OS/process-safe lease or a bounded SQLite read transaction. A segment cannot be retired until all readers with an applicable snapshot are released. Readers must not depend on a lease that can silently expire mid-read without notice.
- Compaction writes a **new sealed immutable segment** containing equivalent canonical payloads (possibly reencoded or deduplicated), validates it, fsyncs it, atomically changes object-location rows, then waits for older readers before deleting old segments. Old and new physical payloads have the same canonical ObjectIds. Compaction failure leaves old location authoritative.
- GC uses a mark-and-sweep root set of all retained authored refs, pinned checkpoints, active unfinished work, materialization receipts/before-images, exports in flight, remote outbox roots, and active query leases. Separate **unreachable** payloads from promised-but-not-local remote objects. Do not use non-owning change lineage or mere mention in an audit report as an indefinite retention pin.
- Mark over exact owning edges with resource-bounded resumable traversal and protected-root generation fencing. If roots change while a sweep runs, restart/extend marking or fence all newly added roots before retirement; never collect on an obsolete root snapshot alone.
- Quota exhaustion or missing payload during a protected operation **refuses before destructive filesystem writes**. Background cleanup may not silently weaken an acknowledged explicit pin. Retention policies are disclosed; no claim about indefinite retention after an acknowledged expiration.

**Checkpoint defaults:** explicitly pinned checkpoints stay until explicit discard; automatically generated *routine* protection may expire after **seven days**, but only when its exact expiry was disclosed at acknowledgement. Preserved unfinished work survives ordinary checkpoint expiry until successful verified resume or explicit discard. The user can inspect and extend protection; GC of an expired checkpoint doesn't delete bytes still protected by another root or export. Change history retained by refs stays until refs/pins are deliberately moved/removed and GC actually reclaims it.

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

Undo checks exact current scope/generation and applies an inverse *view* transaction; if other independent work advanced, refuse rather than applying to a guessed target. An undo of a commit/amend does not erase immutable objects, remote publication or arbitrary current filesystem edits. Returning to an unfinished effort and restoring its bytes is different from historical revision navigation.

Stable idempotency keys are per-request/scope, not universally reused across different plans. Repeated requests return the committed previous outcome or a typed still-pending state; do not repeat an ambiguous destructive operation after a crash or network loss.

## 8. Export, backup and disaster recovery

A **native self-contained export** chooses exact roots/ref generations and an explicit retention/history scope, pins them before traversal, streams the **complete owning closure** into a versioned manifest + content-addressed object stream, and verifies every canonical ID and required relationship. Export is independent of the source storage backend and must be restorable with no Git or network service. Unknown promised/missing objects cause a refusal or a clearly labeled **partial transfer**, never a successful full backup.

An export manifest includes format/protocol version, chosen refs/roots, source repo instance ID, exact protected histories, foreign import metadata when selected, object count/checksums, and proof that EOF/manifest was completed. Do not include excluded private checkpoints or credentials by default. A backup includes mutable local refs/operation state only if explicitly requested; restoring shared Git identities or local filesystem locations may require a separate remap and does not grant publication permissions. An object-only export is not a complete workstation-environment snapshot.

Validate backup by independently restoring into an empty store and checking source bytes, revision ancestry and manifest integrity; then run representative local clone, read and diff without contacting the source. Snapshotting only `store.sqlite` or only `objects/sealed` is not acceptable.

### Experimental object-only source stream

The current `export`/`import` interface is **not the full backup manifest above**. It exports one exact source root (Map or root Conflict), its complete owning objects, and no local selections, request receipts, private discarded roots, repository instance, workspaces or authored refs. The current source-only selection cannot point at a Revision. Source-level restore is independent of the originating SQLite/files and does not require Git/network.

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

## 9. Engineering/qualification gates

A Rust implementation needs deterministically injected failures at every file create/write/sync/seal, SQLite commit, rename, compaction location swap, reader release, GC deletion, materialization step, backup and restore stage. Check power-loss persistence with platform-specific tests **and** ordinary process crash/cancellation tests, not only symbolic lifecycle models. Verify against a second, SQLite-only durability control; retain the simpler implementation if hybrid costs outweigh real benefits at equivalent guarantees. Security checks include untrusted compressed lengths, path traversal, symlink parent attacks, permission failures, crash immediately after acknowledgement and stale readers.

**No claims of source custody until the real CLI entry point and independent restore pass these gates.**