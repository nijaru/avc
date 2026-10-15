# AVC native VCS — authoritative v0 architecture and format contract

**Decision:** 2026-10-15. **Status:** design-finalized for native v0 engineering, not implemented, proven optimal, or released as a stable byte format.  
This document is the single authority for v0 logical source, immutable objects, revision semantics, workspace/recovery state, merge behavior, persistence, native exchange and publication. [Research](research.md) records reasons and rejected options; [qualification](roadmap.md) records tests and release gates.

**Rule for change:** Do not change the model because another VCS optimizes a different workload. A correction needs a demonstrated invariant failure, an essential workflow it cannot support, or compelling comparable end-to-end evidence. Implementation cannot silently redefine these contracts. Byte-level codec details below are **proposed v0 constants**, which must pass independent golden-vector review before the first durable native repository is created. That gate is verification of this chosen design, not permission to improvise a different architecture.

## 1. Purpose, non-goals, and invariants

AVC is a native, general-purpose, local-first source VCS for ordinary editors and terminals, laptops/desktops, offline development, persistent VPS/SSH workspaces, remote/cloud environments, ephemeral agent containers and microVMs, CI/build workers, and scalable source-query services.

Source history, mutable workspace bytes, private recovery, authored revisions, shared publication, build/test evidence, and runtime/deployment images have **different owners and lifetimes**.

The mandatory user-visible invariants are:

- **I1 Exact state:** Each admitted immutable StateId reconstructs exactly one tracked, typed logical source namespace, independent of ancestry, author, cache, physical packing and editing history.
- **I2 One source authority:** Source bytes are identified by immutable Merkle objects. Changeset/patch records, diff indexes, copy hints and provenance are not competing authorities.
- **I3 Honest durable acknowledgement:** Acknowledging a State, Revision, Capture or remote draft root means all canonical object dependencies and necessary root/pins are durable within its stated failure and retention domain.
- **I4 Independent work:** Unrelated workspaces and private captures may advance without changing a global repository head or blocking another workspace's logical selection. Publication to the same target is conditional and ordered.
- **I5 No false filesystem ownership:** A normal filesystem directory is writable by external processes. Status, capture and in-place restore never claim an atomic instant or absence of races without a real controlled-writer boundary.
- **I6 Lossless uncertainty:** Unknown/ambiguous merge results preserve relevant original source alternatives as typed conflicts; a clean merge does not mean semantic correctness.
- **I7 Offline source custody:** A full native repository can inspect/edit/backup without Git, daemon or network. An independent native export/backup can reconstruct retained source without hosted service.
- **I8 Sparse accuracy:** A missing promised object is never interpreted as absent source. A thin client with insufficient proof may stage edits but cannot assert a complete new StateId.
- **I9 No implicit execution:** Parsing or materializing untrusted source never runs hooks, downloaded code, merge drivers, agent tools or scripts without explicit authorization.

No requirement for Git object/pack/ref/protocol or interoperability, causal patch algebra, hosted forge, agent orchestrator, virtual filesystem, process snapshots, build engine or deployment controller. A later adapter is free to exist **outside** native canonical semantics.

## 2. Chosen data model and object graph

The immutable data graph has typed objects; mutable names and pointers live in a separate transactional authority.

    BlobId       -> exact byte sequence
    MapId        -> deterministic directory map of name -> Entry
    ConflictId   -> unresolved source alternatives, possibly at a subtree
    StateId      -> RootEntry (Directory, or unresolved directory Conflict)
    RevisionId   -> StateId, ordered parents, ChangeId, authored metadata,
                    optional immutable provenance reference
    ProvenanceId -> references to INPUT revisions, conflict occurrences,
                    merge/structural recipe details, no output self-reference

    Entry = File { blob: BlobId, executable: bool }
          | Symlink { target: BlobId }
          | Directory { children: MapId }
          | Conflict { value: ConflictId }

    RootEntry = Directory | Conflict of directory alternatives
    ChangeId  = 128-bit cryptographically random logical authored identity

**State identity versus history identity.** A StateId describes the source value alone, including typed unresolved expressions. Two different authors or histories can create the same StateId. RevisionId includes that StateId, ordered parent RevisionIds, ChangeId and immutable author/description/time/provenance. Two Revisions can have equal State and different IDs. ChangeId normally persists across amend/rebase and does **not** imply a context-free patch, one-time application, or immunity to diverging rewrites.

**Revision DAG:** zero parents is a root; one parent is normal; 2–64 ordered parents is an authored merge. Ordered parents express history/provenance and determine displayed first-parent history, but **do not reconstruct source**. A merge Revision explicitly names its full resulting StateId. A revision may not reference itself or a descendant. The source graph is an acyclic graph of immutable object dependencies.

**Evolution:** Rebase/amend/split/fold produce new RevisionIds and optional evolution records relating predecessor/successor revisions. Old revisions remain intact for as long as retained roots, recovery policies or references protect them. A newly authored independent change receives a new random ChangeId; a rebase/amend retains its old ChangeId; split/fold yields explicit many-to-many evolution plus specified new ChangeIds, never guessed from timestamps.

**Provenance acyclicity:** A Revision may reference an immutable ProvenanceId. That provenance may name only input/ancestor revisions, source values, file paths and recipe identifiers, never the output RevisionId itself. The output owning Revision is known by the reference to provenance. No hash self-cycle, and authorship/provenance must never enter StateId.

## 3. Canonical v0 object encoding and IDs

**Chosen hash:** 32-byte BLAKE3 digest, object-kind separated. Canonical payloads use a single deterministic binary codec, not Rust struct layouts, JSON, CBOR implementation defaults, filesystems, SQLite page bytes, or physical compressed streams.

For every kind:

    ObjectID(kind, payload) =
      BLAKE3-256(
        bytes("AVC-OBJECT\0v1\0")
        || u8(kind)
        || u64le(payload_length)
        || payload_bytes
      )

The identifier displayed externally contains format/hash family and full digest, conceptually avc1-b3-<lowercase-base32-no-padding>. Short forms are UI abbreviations, never network object identities. The object kind must be checked on every reference even if a collision is computationally implausible. An object is rejected if the canonical decoder would re-encode it differently.

Primitive grammar:

- **u8, u16le, u32le, u64le, i64le:** fixed-width, little-endian. bool is exactly 0 or 1; no floats. A path name is a length-prefixed raw byte string; all byte strings have explicit lengths.
- **byte-string:** u32le length followed by bytes, unless a particular field uses u64le for large file content. No Unicode normalization; invalid UTF-8 is valid content/name subject to pathname rules.
- **optional:** u8 presence flag 0/1, followed by its field if present. Arrays: u32le count then contiguous canonical elements; reject unknown tag, trailing bytes, duplicate entries and noncanonical ordering.
- **object ref:** u8 expected kind followed by 32 digest bytes. A mismatched kind is a structural error. Maximum nesting/depth and total expansion are enforced independently of encoded lengths.
- **Blob (kind 1):** payload is exactly raw file/symlink byte content (u64le length lives in the common envelope); a huge blob is streamed and never needs to fit memory. Content chunking/compression are *physical* only.
- **MapLeaf (kind 2), MapBranch (kind 3):** specified in section 4; both are MapId family, with their actual typed kind encoded in references.
- **State (kind 4):** one RootEntry descriptor, no author, history, timestamp or path-index cache.
- **Revision (kind 5):** 32-byte StateId reference; u32le parent count and ordered 32-byte RevisionIds; 16-byte ChangeId; author as byte-string; description as byte-string; i64le authored Unix nanoseconds; optional ProvenanceId. UTF-8 may be required by a UI but canonical bytes are retained. Signatures and mutable review notes are separate immutable attestations referencing RevisionId.
- **ConflictValue (kind 6):** specified in section 6; it identifies source alternatives, not authorship/merge engine.
- **Provenance (kind 7):** canonical list of input revision IDs and source/conflict occurrence records; recipe/driver type and exact version. No output RevisionId.
- **ConflictGroup (kind 8):** deterministic fixed-fanout, sorted immutable references preserving all overflow/conflicted input roles without importing authorship into source identity. Exact normalization and role encoding are specified by the same v0 canonical format.
- **Other immutable protocol/backup objects:** receive distinct kind tags, never reuse historical tags.

The object envelope is the *hash input*, not a claim that physical storage writes one file per payload. A MapId reference carries the **actual** MapLeaf or MapBranch kind tag; either is valid only in a map-reference slot. A State root carries an Entry descriptor constrained to Directory or a Conflict whose alternatives are directories. A decoder must reject pathological allocations before allocating by an untrusted length. Different object-kind registries and future codec revisions require **new IDs**; old decoders and explicit ID mappings remain supported for histories that have been published. Hash IDs verify integrity, **not authenticity, authorization, secret protection or object availability**.

## 4. Source namespace and deterministic Merkle map

Canonical file types for v0: regular file with exact Blob bytes and executable bool; symbolic link with exact target bytes; directory, including empty; and a typed unresolved conflict. Do not canonicalize mtime, owner, OS-specific ACL/xattr, hard-link identity, device nodes, generated file contents or line endings. Unsupported admission is an explicit error, not lossy conversion.

Path components: nonempty, raw byte names, no NUL or slash, not "." or "..". v0 upper bound **255 bytes per component**, **4096 path bytes** including separators, **256 components**. A directory cannot also be a file/symlink. Reject duplicate child names. Case/Unicode normalization is **not** part of source equality; materialization on a filesystem unable to represent names must fail safely and preserve original source. Symlink target content is data; never follow an untrusted link when materializing the tree or checking containment.

**Choose a hierarchical Merkle directory tree whose each child map is one deterministic path-compressed radix map over byte-name nibbles.** The map is independent of insertion order. The particular algorithm is fixed for v0:

1. Encode each child name as a sequence of high/low four-bit nibbles. The entry descriptor is one tag plus its BlobId/MapId/ConflictId and optional executable flag. At a map trie position, keys are remaining nibble suffixes.
2. **Leaf:** if the canonical leaf representation of all remaining entries at this position fits **4096 bytes**, encode exactly one sorted leaf. Leaf payload is u32le entry count followed by records sorted lexicographically by full recovered byte name: u16le suffix nibble count, packed high-nibble-first suffix bytes (zero unused low nibble), then canonical Entry descriptor. Empty map is one empty leaf. No leaf may exceed 4096 encoded bytes.
3. **Branch:** otherwise take the **longest common remaining nibble prefix**, encode it once, then create a terminal Entry if a name ends there and child MapIds for nonempty next-nibble groups. Payload is: u16le prefix nibble count, packed prefix, u8 terminal-present, optional Entry, u16le bitmap, then child kind+hash in ascending nibble order. The branch consumes that prefix plus one nibble on each child edge. At least two terminal/child outcomes are required; one-child chains are path-compressed. Child groups recursively apply the same leaf/branch rule.
4. **Canonical normalization:** if a branch's entire key/value set would fit in a leaf, it must be a leaf. A branch's prefix must be maximal. Empty children, uncompressed one-child chains, overlapping ranges, duplicate keys, bad nibble padding, or inconsistent subtree membership are rejected. This makes identical maps yield identical encodings **regardless of editing/update order**. An update at a threshold may rewrite the affected local subtree but not unrelated sibling directories.
5. Directory entries reference a child map; relocation/reuse of an unchanged directory subtree changes only its ancestor links. A map is immutable; range scan/lookup/diff use nibble order and Merkle subtree equality. Sparse membership and absence witnesses must include the full trusted ancestor path and any relevant leaf/branch boundary proof, not just an unverified "not found" response.

**Name encoding example:** both names "a" and "ab" can coexist. An ending name is a terminal at its path; it must never be confused with a longer name sharing its prefix. Odd-length nibble prefixes carry a zero unused padding nibble and must be validated. Separate directory levels prevent confusing a filename containing slash with nested paths.

Why not a flat single directory blob? Wide directories would rewrite it on every small edit. Why not canonically require a content-defined POS/prolly B-tree? Ordered scans and page packing can be excellent, but source path-prefix lookup, subtree moves, and deterministic upper fanout support this radix choice with fewer boundary-shift dependencies. **Physical** cache pages/batches and optional query indexes remain free to use different layouts without changing StateIds.

The **4096-byte leaf target**, nibble branching and codec above are v0 format choices. They are not performance facts and must pass actual golden vectors, adversarial-wide-map and byte-size checks before the format becomes usable. After release, tuning physical packing must not silently change canonical hashes.

## 5. Constructing and observing source

Source construction is a pure conditional transform against an immutable base StateId:

    Apply(base_state, requested_edits, expected_path_witnesses)
      -> either CompleteStateId or typed Conflict/Error

Edits: Put(path, expected_previous_or_absent, new Entry), Remove(path, expected), CopySubtree(source typed immutable subtree value, destination, expected_destination), and MoveSubtree(source path, destination, expected_source, expected_destination). The source of a Copy is a content-addressed immutable value, **not a reference to an otherwise-dead historical revision**. A move is the corresponding source binding removal and destination insertion with value reuse, checked atomically.

All expected path/absence conditions refer to the pinned **base**. Structural operations are resolved from the base first, with no operation reading another operation's in-flight output. Leaf edits may explicitly target the post-structural names with separate expected entries in that constructed structural view (e.g. move src/ → lib/ then edit lib/x). Reject cycles, ancestor/descendant overlap, mismatched old entries and ambiguous contradictory destinations before admission.

The API can preserve provenance/hints about a user-commanded move. A filesystem observer often sees only deleted/created paths and cannot infer unique rename intent. Exact one-to-one subtree/file-content matching can provide a **verified move hint**; heuristic similarity is advisory, never source authority. An expensive rename or directory move is cheap *when* the immutable subtree value is known. A move-and-edit performed invisibly by a third-party editor may require more changed paths or conflict conservatively.

**Normal filesystem capture:** tracked/admitted files are enumerated and hashed; old immutable base provides unchanged path values. Watchers, stat checks, double scans and mtime caches are hints. Without exclusive writers or a real filesystem snapshot, even two successful scans do not prove that all captured files coexisted at one point in time. An authored State can still identify exactly the bytes AVC admitted; it must label observation scope/consistency instead of lying about a live-directory instant. A user-selectable partial-path author operation uses unchanged base for unspecified paths and never silently includes secrets or ignored/untracked files.

**Partial client:** the authoring service verifies exact base State and touched paths; or the client supplies authenticated Merkle membership/absence witnesses and obtains the resulting root independently. If neither is possible while offline, the client may durably queue a **pending edit bundle** relative to a base, not claim a validated complete StateId. A separate local full clone always supports self-contained offline creation.

## 6. Merge and conflict semantics

**Default:** conservative three-way source-state merge. It constructs a new State and an immutable merge recipe/provenance record. For fixed B/L/R values and exact algorithm version, the result must be deterministic; it is not guaranteed semantically correct or associative. **Text tie-breaking v0:** among shortest edit scripts of the same length, prefer the earliest unchanged base-line anchor and then deletion over insertion, comparing inserted raw lines bytewise. Source encoding and overlap checks must have golden vectors; no platform-specific diff library's default order is canonical.

    if left == right:                  return left
    if left == base:                   return right
    if right == base:                  return left
    if left/right are directories:     merge corresponding child names
    if supported file text and edits provably disjoint:
                                      apply deterministic line/hunk merge
    else:                             create an unresolved ConflictValue

For text v0, auto-merge **valid UTF-8** files up to **1 MiB** with no ambiguous overlapping hunk changes. Split lines retaining original line terminators; choose a deterministic Myers shortest-edit script with specified lexicographic tie breaking. Identical overlapping replacements collapse once; conflicting overlapping edits or different insertions at the same anchor remain unresolved. Exceeding CPU/memory/line-count limits returns typed conflict, not partial output. Binary/invalid UTF-8 files, symlink targets and incompatible mode/type changes are only resolved by trivial equality/base rules unless an explicitly authorized deterministic driver is selected. Automatic structural/AST drivers are **not required for v0**; syntax or model-generated merge is an auditable proposed resolution, not hidden canonical decoding.

**Rename-aware behavior:** a VCS-recorded source Move or a one-to-one unambiguous exact-content inferred move can guide an edit-vs-move merge. Two destinations for the same old source, delete-vs-rename, ancestor overlap, or similar-content heuristics that cannot determine identity become typed unresolved conflicts. Do not drop a branch's edits merely to make the result clean.

**Multiple merge bases:** compute the set of *maximal common ancestors* in the revision DAG. One base: ordinary 3-way. Multiple: construct a deterministic, memoized **conflict-preserving virtual base** by recursively merging base revisions in sorted full RevisionId order using the same rules, retaining all unresolved semantic alternatives; record the base frontier and exact recipe. Recursion/expansion limits cannot erase alternatives: if they are exceeded, preserve an opaque unresolved group containing references to the candidate base/side trees. Never arbitrarily select one base or pretend sequential order is semantically associative. For n-way proposal composition, record explicit pairwise merge order; reject "order-independent result" claims.

### Conflicts are typed source values, not marker files

    ConflictValue {
      base: optional EntryValue,             // absence is a real term
      alternatives: sorted distinct EntryValues (2..64),
      kind: PathValue | CoupledSubtree,
      opaque_nested_terms: optional immutable references
    }

The canonical ConflictValue contains **only unresolved source semantics**; source EntryValue can be absent, File, Symlink, Directory or an earlier ConflictValue reference. The outer StateId includes the conflict value; equal unresolved semantic values have equal source hashes. Source conflicts can be file, path, directory, binary, delete/edit, and **coupled across multiple paths** (e.g. divergent renames).

For a multi-path coupled conflict, place an Entry::Conflict at the **lowest common ancestor subtree** containing all coupled paths. A root-level coupled conflict is representable because State can hold a conflicted root of directory alternatives. Alternatives are immutable subtree values, not fully copied directory trees. Reads below the unresolved region **traverse each alternative**: when every alternative has the same typed path value, return that agreed value; otherwise return typed unresolved alternatives. A sparse client must possess proofs for the relevant alternatives or request the service. Never silently choose one branch. This retains normal access to unaffected sibling paths even when the common-ancestor conflict is high in the tree.

Normalization rule: equal side entries are one resolved value; unchanged-vs-changed selects changed; otherwise sort alternatives by canonical typed-value encoding, deduplicate *value-equivalent* terms, preserve distinct bases and nested conflict references, and collapse a ConflictValue to a resolved Entry only when one source value is unambiguously left. Do **not** flatten arbitrary nested conflict expressions, cancel terms across unrelated ancestors, or truncate alternatives when bounds are hit. If an expression exceeds v0 depth/side limits, encode **opaque conflict groups** as a deterministic balanced tree of kind-8 ConflictGroup objects: sorted child content IDs, each node bounded to 64 refs, and distinct base and alternative roles. This tree retains references to every original input and produces a typed complexity error on further automatic resolution. No discarded alternative is permissible; a complexity fallback must still have an unambiguous canonical hash.

**ConflictOrigin/Provenance** is *separate*: input RevisionIds, relevant paths and side roles, base frontier, exact driver/recipe version, move hints, and recorded resolution choices. The output Revision owns a ProvenanceId; the provenance never names that output RevisionId (no reference cycle). Two revisions may contain an equal conflict source value with different causal explanations. v0 is not a causal patch algebra: deduplicating identical source terms is a **source equality choice**, not a claim of causally correct selective undo.

**Presentation:** conflict marker files are optional, generated lossily for an ordinary editor. A manifest binds marker bytes+path to source ConflictId and presentation codec. An unchanged marker file remains unresolved on import; changing text does *not* implicitly and safely resolve a conflict unless the user explicitly confirms/records a validated resolution. Binary/directory/coupled conflicts may have no text-marker presentation. Resolve by choosing or constructing new Entries; prior conflicted input States and provenance remain inspectable.

## 7. Rebase, amend, undo, revert and change identity

**Rebase** of single-parent Revision C with old parent P onto new base Q is a defined three-way source transformation:

    outcome = Merge(base=State(P), left=State(Q), right=State(C))
    C' = new Revision(state=outcome.State, parents=[Q],
                      logical_change=C.logical_change, ...)
    Evolution(C -> C', Rebase)

The result may be conflicted; old C remains immutable. An explicit selected stack is rebased in topological order, mapping already-rebased parents. Do not automatically rewrite unrelated descendants or accepted targets. Rebase of a multi-parent merge requires an explicit parent-mapping/merge recipe; v0 **rejects unsupported merge-commit rebase** rather than guessing.

**Amend** creates a new Revision and retains logical ChangeId; other descendants of the old version may still exist. **Undo** restores private selection/capture/operation roots by scoped generation checks, not an uncontrolled filesystem overwrite. **Revert** constructs a new inverse-*content* change relative to its parent and applies that against current State, possibly conflicting. **Reapply** intentionally performs a previously reverted transformation on a new base. An everlasting "seen ChangeId" blacklist is forbidden; identical content in separate histories is not automatically already merged.

Content equality, revision identity, logical ChangeId, operation receipt, merge provenance and validation evidence are independent concepts; no single ID conflates them.

## 8. Workspace state, recovery and materialization

One independent writer gets its own **writable directory**; immutable source and authorized cache entries can be shared. Workspace ID, actor/session ID, process/VM ID, root selection and on-disk inode identity are separate.

Three mutable resources, with **independent version/generation**:

    Selection { workspace, generation, selected_RevisionId }
    CaptureHead { workspace, generation, latest_CaptureId }
    Materialization {
      workspace, generation, intended_StateId, destination_identity,
      phase: Preparing | Writing | Verified | Incomplete,
      operation_receipt
    }

A status call is a **fresh observation**, not a permanently correct Clean flag. It records observed dirty/new/missing paths, scan failures and watcher uncertainty, plus selected revision/generation. Read-only status is read-only by default.

**Recovery Capture** is a *separate short-lived authority*:

    Capture {
      base_StateId, admitted_bytes_and_deletions, covered/failed/skipped_paths,
      consistency: PerFileObserved | WriterQuiesced | FilesystemSnapshot,
      durable_receipt, created_at, retention_deadline
    }

Capture may use immutable overlays/deltas against a pinned base with bounded replay depth. It is not an authored Revision and does not trigger public share, CI, review or target movement. Only bytes actually admitted and acknowledged are protected. Capture triggers: explicit request or explicitly configured editor/agent boundary; a watcher/daemon is optional and must report missed-event gaps. A capture's original base and all objects stay pinned until its promised retention expires or the user explicitly promotes it. If a concurrent selection change makes a capture outdated, keep its acknowledged bytes as historical recovery rather than retarget it. The capture head can fail to advance on CAS, while the separately acknowledged capture remains pinned. Distinct selection and capture generations eliminate false logical conflicts.

**Default retention:** unpromoted private captures seven days; unpinned orphaned authored revisions ninety days; accepted or explicitly pinned roots while selected/referenced, subject to deliberate retention policy and backup. These are v0 defaults, not claims of infinite durability. Quota exhaustion must reject new captures clearly, not delete older still-acknowledged captures without respecting their horizon.

**Two materialization contracts, for usability and safety:**

- **Fresh-directory restore / new workspace** is the default for recovery and agent branching, and the only portable strongest no-clobber path. Allocate a new empty destination, record durable intent, construct files with dirfd-relative safe operations, verify them and mark ready. Never overwrite the original working directory.
- **Ordinary in-place switch** is supported for day-to-day developer ergonomics, *not* promised to be atomic against arbitrary external writers. If the observed workspace is dirty or watcher coverage uncertain, refuse unless the user explicitly chooses a stronger managed-writer mode or preservation policy. Before mutating any file, admit a protected recovery image of the relevant observed source; record an intent journal, write/rename guarded temporary replacements, verify and report exact progress. A race from an uncontrolled editor between observation and mutation may still exist and must be disclosed; it cannot truthfully receive the strongest no-clobber guarantee. A managed writer-exclusion boundary (or an OS/filesystem snapshot with appropriate coordination) can strengthen that guarantee.

OS check-then-rename does not revoke open descriptors. Materialization can use CoW/reflinks/overlay/VM snapshots as optional accelerators; none is required. Filesystem links cannot escape the destination during path-safe creation. A crash leaves recoverable before-images and an explicit incomplete outcome, never a fictitious complete checkout.

## 9. Durable storage, crash ordering, GC and backups

Local authority is **SQLite** using WAL and a durable synchronous setting for acknowledged writes (reference: synchronous=FULL on a supported local filesystem). It holds small immutable objects, workspace/target roots, generations, operation IDs/receipts, retention pins, reader/GC epochs and object-staging metadata. SQLite WAL is local-host storage; do not claim it works across an arbitrary network filesystem or constitutes a distributed consensus service.

Large immutable blobs/physical segments live as content-addressed files. Two-phase admission:

1. Reserve a durable staging identity/pin before storing large objects. Hash and verify immutable bytes and all required canonical dependencies.
2. Write to fresh temporary content paths, flush contents, atomically link/rename into immutable storage and sync directory metadata as guaranteed by that filesystem. Failed syncs mean **no acknowledgement**. Protect identical-ID uploads against hostile mismatched data.
3. In one short SQLite transaction check exact resource generations, authorization/policy epoch, complete object closure, operation idempotency key/epoch and pins. Publish any new root, receipt and associated events. Only **after** durable commit can AVC acknowledge authored source or capture.
4. Crash before commit: old roots remain; staged objects are reconciled or eventually collected. Crash after commit: root references protected complete immutable closure. Lost network reply: return committed receipt for identical request key; never repeat side effects.
5. Readers pin an immutable authority snapshot/epoch. GC marks all retained roots (history, private captures, drafts, accepted targets), staging and active readers; reclaims only after retention and epoch fences. Physical compaction never changes canonical IDs and cannot free readers' bytes.

No cross-store atomicity is assumed; the ordering and recovery protocol provides the one-directional safety property **"committed root never points at unprotected missing source."** Backups capture a consistent set of mutable roots plus all referenced immutable closure, with a replayable manifest and corruption check. Missing data cannot be fixed from its hash alone. Declared durability means only the underlying host/filesystem/replica guarantees; hardware that lies about sync or loses all replicas is not repaired by content addressing.

**Physical layout choice:** initially whole logical blobs, small SQLite object records and large immutable files. Later byte compression, content-defined chunking for huge edited assets, bounded delta chains, packs, cache pages and multi-region tiering may optimize access without altering BlobId/StateId. Do not canonically require Enroute's indexed layers, Git packfiles, global dense IDs, Raft or S3. Physical optimization is evaluated under identical durability, cold/hot reads, GC cost and transfer patterns.

## 10. Native protocol, hosting and publication

One source-operation contract works as a local library and a networked service. Operations include typed GetState/GetPath/GetFileRange, ListDirectory, DiffState, ancestry/history/blame query (derived), conditional Apply, CreateRevision, Merge, FetchObjectBatch, native bundle exchange, PinDraft and target proposal/acceptance. All requests are authenticated/authorized, bounded, versioned and idempotent where mutating.

**Full client:** holds the object closure for selected retention roots, can author/inspect/merge offline. **Partial client:** explicitly records promised missing objects, hydrates by exact ID/path witnesses or asks service to build verified source. **Thin remote client:** may query/compose without any working tree checkout. **Bulk transfer:** compressed, resumable selected-object closure/history bundles avoid one network RTT per node; sparse exact-object queries avoid shipping whole history. No native Git format requirements.

**Ephemeral agent invariant:** a microVM/container may be destroyed only after the receiving service acknowledges *all source objects and a pinned immutable draft root*. Uploaded unpinned blobs or a workspace name without durable object closure is not a successful handoff. Worker runtime/VM state, toolchain cache and generated binaries are not source history.

**Publication candidate** binds target name and observed target generation, selected RevisionIds, merge recipe, resulting exact StateId/RevisionId and policy digest. Validation evidence is **a separate record referring to that candidate**, with exact test/toolchain/source inputs; it must not create a candidate hash cycle. Tests and review run outside the target transaction. Accept transaction checks expected target generation, authorization, policy epoch, evidence matching the same candidate, and complete immutable closure; it conditionally advances only that target. If target changed, recompute/revalidate as policy requires; preserve the failed proposal and its source.

Default protected target: **validate before acceptance**. An explicit high-throughput policy may separate accepted and verified target roots, with deployment gated on verified. A clean text merge is never sufficient evidence of program correctness. Long-lived deployment artifacts carry the exact source RevisionId, build recipe and validation provenance; production processes need not host AVC.

**Security and multi-tenancy:** knowing a content hash does not authorize reading it. Object-existence tests, path-scoped proofs, CAS miss responses and shared caches cannot leak cross-tenant data. Untrusted repository content may not auto-run hooks/merge programs. Authenticated authorship and signing are external attestations; hashes only establish byte integrity. Track/ignore policy must be explicit—adding an ignore pattern later does not remove committed secrets from retained immutable history and backups.

## 11. Adversarial invariants and decisions

| Failure/edge case | Mandatory behavior |
| --- | --- |
| Equal source from different edit orders/histories | Identical StateId under v0 codec, possibly different RevisionIds |
| Tiny edit under a million names | Update affected Merkle map path only; no unbounded whole-directory rewrite |
| Directory rename + edit, or copy from old branch | Content subtree reuse, no permanent old-revision dependency for copied bytes |
| Two names differ only by case on case-insensitive OS | Source retained; unsafe materialization fails without silent rename |
| Two sides rename one file differently | Coupled subtree conflict, preserve both destinations and original values |
| Criss-cross merge/previously conflicted base | Memoized deterministic virtual base, preserve unresolved terms, no arbitrary loss |
| Conflict has same semantic alternatives, different authors | Same conflict source value/StateId; separate provenance/RevisionIds |
| Missing sparse path proof | Reject complete StateId; retain pending draft if requested |
| Two workspace selections, watcher capture race | Independent generations, no unnecessary logical conflict |
| Editor writes during capture | Coverage and consistency class disclose gap; no point-in-time claim |
| External editor writes during restore/switch | Fresh-directory restore guarantees no overwrite; guarded in-place switching refuses known dirt and journals observed before-images, but cannot promise no races from uncontrolled writers |
| Crash/GC during staged blob publish | No acknowledged root loses protected object closure |
| Remote agent dies after uploading only blobs | No "saved work" claim until durable pinned draft receipt |
| Target advances after candidate was validated | Conditional acceptance fails; evidence is not silently transferred to new candidate |
| Malicious path/symlink/huge nested conflict | Fail safely; no filesystem escape, code execution, or dropped alternatives |

**Selected architectural constants (v0):** snapshot-root authority; exact State/Revision/ChangeId separation; BLAKE3-256 typed IDs; deterministic nibble radix child maps with <=4096-byte canonical leaves; byte-exact regular files/symlinks/empty dirs; source-only conflict values and separate origins; conservative merge/rebase; private bounded captures; independent per-workspace roots; SQLite transactional local authority plus staged immutable large objects; native full/sparse/thin clients and conditional validated targets; no required Git compatibility.

**Engineering qualification, not new architecture selection:** exact golden vectors and parser limits, conflict normalization fixtures, deterministic text-merge tie-break vectors, filesystem fsync/name portability, and resource/latency tests. If a chosen constant fails, make a documented versioned correction **before any stable repository is created**, rather than silently changing hashes under existing users.

**Current implementation:** none of this native engine is implemented. Rust 0.0.1 remains a Git-command/JSON-log prototype with none of the advertised native guarantees. The research fixtures are not release qualification.


## 12. Normative v0 field order and canonical expansion rules

This section fixes the **object payload grammar** referred to in sections 3–6. Notation: byte[n] is exactly n bytes, ref(k) is u8(actual kind) plus byte[32] digest with kind validation, and str is u32le byte-length plus raw bytes. Hash envelope is defined in section 3 and is not repeated inside a payload.

    Entry =
        0x01 ref(Blob) u8(executable=0|1)      // ordinary file
      | 0x02 ref(Blob)                         // symlink target bytes
      | 0x03 ref(MapLeaf|MapBranch)           // directory
      | 0x04 ref(ConflictValue)               // unresolved source
    MaybeEntry = 0x00 | 0x01 Entry            // 0=absent; 1=present
    StatePayload = Entry                      // only directory or
                                              // directory-valued conflict

    MapLeafPayload =
      u32le(n)
      repeat n: u16le(suffix_nibbles), packed_nibbles(suffix), Entry

    MapBranchPayload =
      u16le(common_prefix_nibbles), packed_nibbles(prefix),
      u8(terminal_present=0|1), optional Entry,
      u16le(child_bitmap),
      refs(MapLeaf|MapBranch) in ascending nibble-slot order

    RevisionPayload =
      ref(State), u32le(parent_count),
      repeat parent_count: ref(Revision) in authored first-parent order,
      byte[16](ChangeId),
      str(author_bytes), str(description_bytes), i64le(unix_ns),
      u8(provenance_present=0|1), optional ref(Provenance)

The MapLeaf and MapBranch **actual kind tags** are distinct in their object IDs. A MapRef uses the appropriate actual tag. Every decoded map is checked against canonical leaf size, maximal shared prefixes, child-map membership, sorted key uniqueness, nibble padding and forced leaf/branch normalization; otherwise it is rejected even if its outer digest verifies. An Entry at a map terminal names the exact key ending at that nibble prefix; it is not a parent directory or a child map in that same per-directory namespace.

ConflictValue has two storage cases:

    ConflictValuePayload =
      u8(region_kind=0 PathValue | 1 CoupledSubtree),
      MaybeEntry(base),
      u8(alt_storage=0 Inline | 1 Group),
      if Inline: u32le(n: 2..64), repeat n: EntryValue
      if Group: ref(ConflictGroup)

Here **EntryValue** is MaybeEntry, so deletion/absence can be an alternative. For Inline alternatives, sort strictly by canonical encoded EntryValue bytes and deduplicate exact equal semantic terms first; a one-alternative result must be represented as a resolved Entry, not a conflict. A RegionKind CoupledSubtree requires that every present alternative be a Directory (or a recursively directory-valued Conflict). The base may itself be an unresolved semantic Entry. Equal base/side normalization and trivially equivalent outcomes apply *before* persisting the conflict. Historical side order is in Provenance, not in ConflictValue.

For more than 64 alternatives, use **ConflictGroup (kind 8)** instead of the inline list:

    ConflictGroupLeafPayload =
      0x00, u32le(n: 1..64), sorted distinct EntryValues

    ConflictGroupBranchPayload =
      0x01, u32le(n: 2..64),
      repeat n: ref(ConflictGroup) in sorted minimum-child-value order

Canonical group construction: sort/deduplicate all alternative semantic values, form consecutive blocks of **64** as leaves (including the final shorter block), then group consecutive up-to-64 child refs by their covered key ranges until one root. Leaf values and child ranges cannot overlap. A branch with a single child collapses to that child. A ConflictValue must use Inline if the number of distinct values is 64 or fewer; otherwise Group, even if a client would prefer another physical layout. This fixes overflow identity independently of insertion order or arrival order. A single nested source ConflictValue is an opaque **value term** for sorting; do not recursively flatten/cancel it without a separately proven equivalence.

**ProvenancePayload** is a canonical versioned *history explanation*, deliberately separate from source values:

    u32le(input_revision_count)
    repeat: ref(Revision) in algorithm-role order
    u32le(occurrence_count)
    repeat sorted by (raw path components, ConflictId):
      encoded_path, ref(ConflictValue), u32le(input_role_count),
      repeat input_role_count: u32le(input_revision_index)
    str(merge_driver_id), u32le(merge_driver_version),
    str(recipe_bytes)

An encoded_path is u32le number of raw components followed by each component as str; its components must pass v0 path bounds. Input roles refer only to **input** RevisionIds. The owning output RevisionId is never embedded and is instead the immutable parent object that references ProvenanceId. Unknown driver IDs/version metadata remain inspectable as bytes but may not be automatically executed. For nonmerge revisions, provenance is absent unless selected copy/move explanations are recorded.

**Canonical size limits:** component/path bounds from section 4; author ≤4096 bytes; description ≤1 MiB; merge recipe ≤1 MiB; a provenance occurrence list ≤1 million entries, streamed and resource-limited; parent revisions ≤64; a source conflict has a bounded **nesting depth of 64** before automatic simplification stops and preserves opaque immutable input values. Admission must use explicit resource budgets smaller than format maxima if necessary and fail without partial acknowledgement. Never replace a representable conflict with a silently truncated one.

**Golden-vector requirement:** independent codecs must agree byte-for-byte and hash-for-hash on empty source, one file, binary content, executable flag, symlink, two prefix-sharing names, leaf/branch boundary at 4096, directory reuse under new name, equal State with different authors, path conflict with absent alternative, coupled root-level rename conflict, grouped overflow conflict and invalid/corrupt encoding. No permanent format claim is made until these are published and tested.
