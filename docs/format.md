# AVC v1 canonical source format and merge semantics

**Status:** selected **design contract**, 2026-11-04. No production implementation, interop deployment, frozen vectors, or source-custody qualification exists. The v1 **logical rules and initial byte grammar** below are the implementation target. If executable conformance finds a flaw before any public repositories are written, revise this single document and regenerate vectors coherently; after release, version and migrate rather than silently rewriting identities. See [design](design.md) for authority boundaries, [storage](storage.md) for durable implementation and [sync](sync.md) for partial clients.

## A. Rules common to every object

- All canonical fields are byte sequences; no Rust struct layouts, JSON, machine endianness, filesystem inode metadata, storage compression, or local path encoding participates in object identity.
- Typed ID: `ObjectId = (kind: u8, hash: [u8;32])`. `hash = BLAKE3("AVC-OBJECT\0v1\0" || kind || u64le(payload_byte_length) || canonical_payload)`. The prefix is the literal ASCII bytes and two NUL bytes indicated. This plain BLAKE3 domain separation is chosen over keyed hashing because the domain includes an unambiguous object kind and version. The digest is **not** an authentication or availability guarantee.
- Wire reference: exactly 33 bytes (`kind || digest`). CLI external spelling: `avc1-kXX-<64 lowercase hexadecimal digest digits>`; `XX` is two lowercase hex digits of the actual kind. Abbreviations are lookup-only and must be unambiguous. No other case, padding or omitted kind is canonical.
- Codec integers: `u8`, `u16le`, `u32le`, `u64le`, `i64le`, and `i16le` as fixed-width little-endian fields. `bytes32` means `u32le(length) || length bytes`, subject to the field's cap. Boolean is exactly 0/1. `ref` is the 33-byte typed ID.
- A tag is exactly one byte. Unknown kinds, tags, unused flag bits, trailing bytes, duplicate map keys, unsorted canonical lists, out-of-range values, malformed length, or nonzero nibble padding must be refused, not coerced.
- A full object decoder re-encodes to the *identical bytes* and checks the typed ID. The physical store may compress, split, pack, and move those bytes without changing the logical ID. A store holding a root must protect all **owning edges** below it.
- Object kinds for initial v1: `0x01 Blob`, `0x02 MapLeaf`, `0x03 MapBranch`, `0x04 Conflict`, `0x05 Revision`. Reserved kinds are invalid until introduced through explicit versioning. No separate `State`, universal `Provenance`, or canonical Patch object.
- All algorithms that traverse untrusted input have caller-configurable resource limits and bounded streaming where feasible; failure to meet a limit yields **no authoritative result**. Never normalize a malformed object into a different ID.

## B. Blob and tree entries

`Blob(0x01).payload = raw bytes`. Its identity is over *exact* bytes, including NUL, CRLF, and encoding. It can represent a file or symlink-target payload; the consuming entry determines interpretation. A Blob may be large; physical chunking never changes its BlobId.

The canonical `Entry` descriptor used in directory maps or conflicts is:

| Tag | Fields | Meaning |
| --- | --- | --- |
| `0x01` | `ref(Blob 0x01) || bool executable` | Regular file; no hidden extra modes |
| `0x02` | `ref(Blob 0x01)` | Symlink target bytes; never follow at source decode |
| `0x03` | `ref(MapLeaf 0x02 or MapBranch 0x03)` | Directory, including empty |
| `0x04` | `ref(Conflict 0x04)` | Unresolved value at this exact path |
| `0x00` | no bytes | **Absent**, legal only inside a conflict expression or a conditional witness; not an ordinary stored directory binding |

A source root is a typed directory map reference, or a `Conflict` reference whose resolved alternatives are directories. Empty source root is an empty MapLeaf. Source root identity is its typed object reference; no wrapper is created merely to repeat it.

Path components are **raw bytes**, not slash-delimited normalized Unicode. Each basename must have 1–255 bytes, not include NUL or `/`, and not equal `.` or `..`. The logical source path has at most 256 components and 4096 bytes including `/` separators. Duplicate names are invalid; names sort by unsigned byte lexicographic order, independent of OS Unicode rules. A platform that cannot represent or distinguish names (case folding, normalization, reserved names) must refuse materialization rather than rewrite them silently. Special files, hard-link identity, ACLs, xattrs, ownership, file timestamps, and device nodes are not in v1 source and require explicit refusal or disclosure at import.

## C. Canonical directory maps

The source namespace is a **hierarchy of per-directory persistent path-compressed nibble-radix maps**. Each directory map binds *one basename* to its typed Entry; it does not flatten path separators across directories. Keys are each basename's high/low 4-bit nibbles, in unsigned-byte lexical order. A map node's suffixes are relative to the prefix already consumed by its ancestors; a MapId alone is not an authority to interpret a name without its complete map-root context.

**Fixed initial leaf rule:** if the complete key set for the current radix subtree contains **0–32 immediate directory entries**, represent it as one sorted leaf. If it contains **33 or more**, represent it as a branch at the *longest common remaining nibble prefix* of those keys. For each nonterminal key, partition on the next nibble (0–15) and recurse. A key that ends at the branch's common prefix is its single terminal; that terminal must end on a complete byte boundary. Do not emit empty child groups or redundant one-child branches. These conditions select one representation independent of insertion order. The 32-entry choice is an initial **format constant** rather than a measured universal optimum.

**MapLeaf 0x02 payload:**

```text
u8 key_count                               # 0..32
repeat key_count in full recovered unsigned-byte name order:
    u16le remaining_nibble_count           # 0..510
    ceil(remaining_nibble_count/2) bytes   # high nibble first, unused last nibble zero
    Entry
```

An empty directory is the one-byte MapLeaf payload `00`. A recovered complete key must end at an even nibble count and pass basename/path checks in the accumulated parent context. Leaf suffix keys must be unique, sorted by complete names, and each reconstructed key must belong to its immediate parent partition.

**MapBranch 0x03 payload:**

```text
u32le immediate_entry_count                 # 33..u32::MAX; sum of terminal + children
u16le common_remaining_nibbles             # 0..510
ceil(common_remaining_nibbles/2) bytes     # prefix nibble packing
u8 terminal_present                        # 0 or 1
Entry if terminal_present=1
u16le child_bitmap                          # nonempty group nibbles, 0..15
for each set bit in ascending order:
    ref(MapLeaf 0x02 or MapBranch 0x03)
```

Require at least two outcomes from terminal plus child groups. Exact maximal prefix and 33+ count are part of canonicality. The `immediate_entry_count` is authenticated as a field within the hashed node. A *trusted validated* child count permits incremental branch→leaf normalization without visiting every descendant; imported unverified counts are never trusted on their own.

Each map node also has a **derived verified metadata cache** (not hashed): exact immediate binding count, maximum relative component depth, and maximum relative path byte length through all file/symlink/conflict alternatives under it. These summaries may be reused only if computed from the exact validated owning closure and codec version; otherwise verify recursively. A subtree move can check new path bounds via these summaries but is not assumed O(1) when summaries are unavailable.

**Incremental editing:** a batch changes exact witnessed paths in a verified root. Clone just affected leaf/branch nodes and directory ancestors; reuse untouched IDs, split/collapse deterministically at the 32-entry rule. Commit one verified resulting root or nothing. Do not reconstruct the whole flat directory map. Diff compares IDs first and expands only changed subtrees or differing radix partitions; a threshold change may require a bounded local reconciliation but cannot drop entries. Refuse cyclic references, wrong kind, invalid bounds, duplicate or overlapping partitions and source that cannot be traversed safely.

**Qualification:** independently construct the same map from sorted, reverse, random, incremental, split/collapse, and batched edit orders; prove byte-equal roots on identical entry sets, and test very wide directories and long common prefixes. A native comparative benchmark may justify altering the 32-entry rule **before first release**, but the implementation must use one documented rule at a time. No fixed v1 object may silently change its bytes after release.

## D. Normalized typed conflict values

**Chosen v1 model:** a *flat signed multiset of exact source entry values* at the conflicted path, similar to Jujutsu's conflict expression but with conservative resolution. It prevents repeated same-path rebases from nesting an unbounded tree of conflict records.

A conflict has a positive list and a negative list of **Entry-or-Absent terms**. Terms are sorted by their exact canonical descriptor bytes (Absent sorts first), then encoded with multiplicity; there may be no entry of the same value with both signs after normalization. Flatten an input Conflict into signed terms before storage, cancel exactly matching positive/negative terms with equal multiplicity, and reject cyclic or noncanonical signed expressions. Terms at this path must be **resolved entry kinds or Absent**, not another Conflict; a directory term may contain conflicts at *descendant* paths.

**Conflict 0x04 payload:**

```text
u8 region                                  # 0 = path entry, 1 = source-root directory
u8 positive_distinct_count                  # 1..64
repeat sorted unique positive terms:
    u8 multiplicity                         # 1..64
    Entry-or-Absent
u8 negative_distinct_count                  # 0..64
repeat sorted unique negative terms:
    u8 multiplicity
    Entry-or-Absent
```

All term multiplicities sum to **at most 64**; total positive multiplicity minus total negative multiplicity equals **exactly 1**. Duplicate entries of the same sign must be combined into one term with multiplicity. Opposite-sign equal terms must cancel, not coexist. A resulting expression with one positive and no negatives is immediately stored as that normal Entry, never an unnecessary Conflict. When `region=1`, every present term must be directory-valued and Absent may not be the sole result. A path conflict's type/mode/binary/absence alternatives are all retained as opaque values; authorship/provenance labels live in revision/operation metadata rather than contaminating source-value identity.

**Three-way combination:** for exact base B, left L, right R, form the algebraic expression `L + R - B`; if any operand is itself a conflict, substitute and flatten its terms (including sign inversion when used as a negative operand). Exact term cancellation is the *only unconditional algebraic simplification* on an unresolved expression. Never automatically discard a nonidentical term via a same-change heuristic. For a small well-defined clean three-way value, the rules in §E may return an exact Entry and the operation retains its input/source comparison metadata separately.

If an expression would exceed the term/resource bound, return `MergePendingTooComplex` and protect its input revisions; **do not invent a resolved value or drop source alternatives**. Incomplete data, I/O, memory, or deadline failure is also pending/aborted, **not** a semantic conflict. Deep directory-valued alternatives are protected through their immutable references; no arbitrary input is copied as marker text.

A conflict is a genuine value: it can appear in an authored revision. Materializing an unresolved conflict into an ordinary filesystem requires a **separate nonauthoritative presentation** (e.g., markers in a temporary file plus an association to its exact original ConflictId). The edited marker file alone is not a valid conflict object; explicit resolution binds a chosen full value to the original conflict version. Git export of unresolved conflicts must refuse or require deliberate explicit resolution, never publish marker text as if it were the native source.

## E. Deterministic merge and evolution

Inputs are **exact immutable revisions/source roots with a complete, provable merge base**, not guessed shallow history. If there are multiple maximal common ancestors, calculate their recursive merge base with bounded work and protect all original bases; if ancestry is incomplete or the computation cannot finish safely, return pending. Automatic merge never establishes program correctness.

For a fixed base B, left L, right R:

1. If L=R, return that exact value; if L=B, return R; if R=B, return L.
2. If B/L/R are directories, merge child names recursively; if B was absent and L/R are both directories, recursively merge independent child adds. Do **not** implicitly union two replacement directories when B was another entry kind. Delete-vs-modify produces a whole affected subtree conflict.
3. If all three are regular files, merge the **content** and **executable bit** independently. Mode follows exact three-way equality/base rules. Content first tries byte equality/base rules; if disjoint text edits are deterministically nonoverlapping under the selected driver, emit the exact combined bytes. A content-only edit and independent mode-only edit can compose cleanly.
4. Initial text driver: `avc.text.diff3/v1`, fixed byte-preserving line tokenizer (LF, CRLF, bare CR terminators retained), patience anchors over unique lines with bounded Myers fallback for gaps. Compare exact byte lines; no Unicode or newline normalization. Identical hunks coalesce. Disjoint replacements and insertions at different gaps compose; overlapping unequal hunks, uncertain competing insertions, or edit-vs-deletion of the same range yield typed conflict, not an arbitrary side preference. Driver output depends only on the input bytes and this version's algorithm, not wall-clock, memory availability or randomized tie breaking.
5. Eligible text files must have valid UTF-8, no NUL, unchanged text mode and at most 8 MiB per input; larger or binary files use exact equality/base rules or an optional explicit external merge that is **not** trusted to define the canonical automatic result. A configured driver can produce a proposed result for review, never silently execute untrusted source-provided code.
6. Incompatible types, symlink targets, binary contents, delete-vs-edit, or unresolved inputs become a normalized signed conflict as in §D. File/directory type conflicts preserve whole subtrees, not partial directory unions. Recorded move intent may guide a checked path remap only when base source and target witnesses still match; similarity detection is a **hint**, not authority.
7. Invalid input, missing promisor object, unknown merge ancestry, or resource exhaustion never yields a fake clean or conflict result. Preserve exact roots and return typed incomplete/pending status.

A versioned merge **recipe** records input revisions, bases, driver IDs, structural hints, and resulting root; it may be attached to an authored operation as optional metadata but is not an alternative canonical source authority. Changed-path logs and conflict provenance outside the result root cannot by themselves retain private source indefinitely.

Amend/rebase produce a new immutable RevisionId while retaining the relevant logical ChangeId. Split/fold allocate new ChangeIds and record precise mapping, not a timestamp-selected successor. Revert is a new authored inverse-content transformation with exact prior source witnesses; history undo is a local view operation and does not mutate previously authored objects. No tool must claim an old revision has ceased to exist just because a new version of its logical change is selected.

## F. Revision 0x05

Payload (field order is normative):

```text
ref source_root                              # MapLeaf/MapBranch or root Conflict
u8 parent_count                             # 0..16
repeat parent_count: ref(Revision 0x05)       # order significant; distinct
16 bytes ChangeId                            # cryptographically random for new authored changes
bytes32 author_utf8                          # max 4 KiB, valid UTF-8
bytes32 message_utf8                         # max 1 MiB, valid UTF-8
i64le unix_time_nanoseconds                  # UTC; timestamps not trusted for ancestry
i16le original_timezone_offset_minutes       # -1439 .. 1439
```

A zero-parent revision is an explicit native history root. Native imports of a *partially known* Git history must mark the foreign ancestry boundary in separate retained import metadata and never imply that the Git commit had no parents; this native root is an **import anchor**, not the authentic Git root. A native revision cannot refer to a foreign Git OID as if it were an AVC RevisionId. Order of multiple parents is intentional first-parent order followed by other parents; parent references are owning edges, unique and acyclic. Changing message/author/parents changes RevisionId; stable ChangeId supports history editing without mutating the old revision.

Immutable detached signatures, external attestations, review/test approvals, and generated changed-path summaries are outside RevisionId. The revision cannot gain new canonical parents after its hash is published; foreign history expansion must be represented through the Git adapter/explicit migration, not a silently changed revision.

## G. API invariants and availability

The native core API works entirely against immutable refs:

```text
read(root, path, policy) -> Known(value) | Absent(proven) | Promised | Denied | Unavailable | Corrupt
edit(base_root, [Put/Remove/Move with exact expected prior values])
    -> PreparedRoot(root, verified_boundary) | Stale | Conflict | NeedObjects | Refused
diff(base_root, other_root, policy) -> path changes or typed Incomplete
merge(base, left, right, driver) -> Clean(root) | Unresolved(root) | Pending(inputs, reason)
```

A complete local authority verifies the required owning closure before acknowledging a durable root. A **native thin client** may recompute a *candidate root* using an authenticated complete base root, complete validated Merkle path proofs, sibling commitments and checked edits even when sibling blobs are absent. It must mark that root **conditionally verified against the base holder's promise**, not locally fully hydrated; the full remote must validate changed dependencies and own the complete closure before publication. This is different from a **foreign Git** partial tree, for which missing native BlobIds have not been computed.

Unsupported operating systems must refuse to materialize names/metadata they cannot preserve. Hash validity is distinct from origin authenticity, access authorization, local availability, consistency across multiple refs and executing source safely.

## H. Format qualification before accepting source

Before this v1 encoding can be frozen: implement an independent reference encoder/decoder; pin empty map, one file, wide directory, split/collapse, symlink, conflict, revision and long-name byte vectors; compare against the Rust engine; run randomized insertion orders and independent flat-tree oracles; fuzz bounded invalid lengths/tags/path and hash collisions; verify integrity after simulated crashes, GC and export/restore. Only then label an immutable format stable. A demonstrated defect in this unreleased grammar should be corrected explicitly rather than hidden behind reader fallbacks.