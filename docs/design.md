# AVC native VCS — authoritative v0 architecture and format contract

**Decision:** 2026-10-15; **reviewed:** 2026-10-27. **Status:** selected v0 architecture under independent contract qualification; not implemented, proven optimal, or released as a stable byte format.
This document is the single authority for v0 logical source, immutable objects, revision semantics, workspace/recovery state, merge behavior, persistence, native exchange and publication. [Research](research.md) records reasons and rejected options; [qualification](roadmap.md) records tests and release gates.

**Change and freeze policy:** The product invariants below are requirements; the selected algorithms and byte format are candidates to qualify against them. Correct demonstrated failures and compare consequential alternatives before committing user data to a stable format. Neither novelty nor incumbent adoption is evidence of superiority. Keep one current contract, update its rationale and fixtures explicitly when it changes, and never silently change IDs for released repositories. Before the first durable native repository, require independent codec/semantic checks and representative layout/merge cost evidence; full production performance qualification follows later.

## 1. Purpose, non-goals, and invariants

AVC is a native, general-purpose, local-first source VCS for ordinary editors and terminals, laptops/desktops, offline development, persistent VPS/SSH workspaces, remote/cloud environments, ephemeral agent containers and microVMs, CI/build workers, and scalable source-query services.

Source history, mutable workspace bytes, private recovery, authored revisions, shared publication, build/test evidence, and runtime/deployment images have **different owners and lifetimes**.

The mandatory user-visible invariants are:

- **I1 Exact state:** Each admitted immutable StateId reconstructs exactly one tracked, typed logical source namespace, independent of ancestry, author, cache, physical packing and editing history.
- **I2 One source authority:** Source bytes are identified by immutable Merkle objects. Changeset/patch records, diff indexes, copy hints and provenance are not competing authorities.
- **I3 Honest durable acknowledgement:** Acknowledging a State, Revision, Capture or remote draft root means all canonical object dependencies and necessary root/pins are durable within its stated failure and retention domain.
- **I4 Independent work:** Unrelated workspaces and private captures may advance without changing a global repository head or blocking another workspace's logical selection. Publication to the same target is conditional and ordered.
- **I5 No false filesystem ownership:** A normal filesystem directory is writable by external processes. Status, capture and in-place restore never claim an atomic instant or absence of races without a real controlled-writer boundary.
- **I6 Lossless uncertainty:** Every admitted ambiguous merged State preserves original source alternatives as typed conflicts; if limits prevent representing that State, refuse admission and retain protected input roots under an explicit pending receipt. A clean merge is not proof of semantic correctness.
- **I7 Offline source custody:** A full native repository can inspect/edit/backup without Git, daemon or network. An independent native export/backup can reconstruct retained source without hosted service.
- **I8 Sparse accuracy:** A missing promised object is never interpreted as absent source. A thin client with insufficient proof may stage edits but cannot assert a complete new StateId.
- **I9 No implicit execution:** Parsing or materializing untrusted source never runs hooks, downloaded code, merge drivers, agent tools or scripts without explicit authorization.

No requirement for Git object/pack/ref/protocol or interoperability, causal patch algebra, hosted forge, agent orchestrator, virtual filesystem, process snapshots, build engine or deployment controller. Git conversion belongs in a separate boundary adapter, not a native Git-compatible storage mode. Conversion must preserve the declared representable subset and report unsupported or lossy cases before writing output; it must not constrain native IDs, conflict semantics or durability. The adapter is deferred, not part of the native engine qualification.

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

    RootEntry = Directory | CoupledSubtree Conflict of present directory alternatives
    ChangeId  = 128-bit cryptographically random logical authored identity

**State identity versus history identity.** A StateId describes the source value alone, including typed unresolved expressions. Two different authors or histories can create the same StateId. RevisionId includes that StateId, ordered parent RevisionIds, ChangeId and immutable author/description/time/provenance. Two Revisions can have equal State and different IDs. ChangeId normally persists across amend/rebase and does **not** imply a context-free patch, one-time application, or immunity to diverging rewrites.

**Revision DAG:** zero parents is a root; one parent is normal; 2–64 ordered parents is an authored merge. Parent IDs must be distinct; order is preserved for first-parent history/provenance, **not** source reconstruction. Every Revision directly names its full StateId; zero-parent roots are separately valid source states. The canonical object-reference graph (including parents and provenance) must be acyclic and checked upon admission (a hash match is not a substitute for graph validation). No unknown, missing or promised parent is treated as empty ancestry. Evolution edges are metadata, not Revision parent edges.

**Evolution:** Rebase/amend/split/fold produce new RevisionIds and optional evolution records relating predecessor/successor revisions. These are local authority metadata, append-only while retained and exported with the retained metadata snapshot, not canonical Revision references or merge ancestry. Validate the separate directed predecessor-to-successor graph transactionally: no self-edge or cycle, and a no-op replacement needs no edge. Do not combine those edges with the reverse references in canonical provenance to invent an object cycle. Old revisions remain intact while roots, recovery policies or explicit pins protect them; an evolution record alone does not extend retention. Its IDs/relationship may outlive endpoint bytes, in which case queries report an expired/unavailable endpoint rather than claiming reconstructability. A newly authored independent change receives a new random ChangeId; rebase/amend retains its old ChangeId; split/fold records explicit many-to-many lineage with newly assigned ChangeIds rather than deriving identity from timestamps.

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

The complete ObjectID is the **pair (kind, 32-byte digest)**. Its canonical external spelling is `avc1-b3-kXX-<digest>`, where `XX` is the two lowercase hexadecimal digits of the actual u8 kind and `<digest>` is exactly 52 lowercase RFC 4648 base32 characters without padding (unused final pad bits must be zero). Short forms are only UI abbreviations. All wire references carry actual kind plus full digest; a digest alone is not a typed ObjectID. Every retrieval verifies the envelope hash, expected reference kind, canonical decoding and referenced closure. Re-encoding a decoded object must reproduce the exact payload bytes.

Primitive grammar:

- **u8, u16le, u32le, u64le, i64le:** fixed-width, little-endian. bool is exactly 0 or 1; no floats. A path name is a length-prefixed raw byte string; all byte strings have explicit lengths.
- **byte-string:** u32le length followed by bytes, unless a particular field uses u64le for large file content. No Unicode normalization; invalid UTF-8 is valid content/name subject to pathname rules.
- **optional:** u8 presence flag 0/1, followed by its field if present. Arrays: u32le count then contiguous canonical elements; reject unknown tag, trailing bytes, duplicate entries and noncanonical ordering.
- **object ref:** u8 expected kind followed by 32 digest bytes. A mismatched kind is a structural error. Maximum nesting/depth and total expansion are enforced independently of encoded lengths.
- **Blob (kind 1):** payload is exactly raw file/symlink byte content (u64le length lives in the common envelope); a huge blob is streamed and never needs to fit memory. Content chunking/compression are *physical* only.
- **MapLeaf (kind 2), MapBranch (kind 3):** specified in section 4; both are MapId family, with their actual typed kind encoded in references.
- **State (kind 4):** one RootEntry descriptor, no author, history, timestamp or path-index cache.
- **Revision (kind 5):** typed ref(State); u32le parent count and ordered typed ref(Revision) parents; 16-byte ChangeId; author as byte-string; description as byte-string; i64le authored Unix nanoseconds; optional typed ref(Provenance). UTF-8 may be required by a UI but canonical bytes are retained. Signatures and mutable review notes are separate immutable attestations referencing RevisionId.
- **ConflictValue (kind 6):** specified in section 6; it identifies source alternatives, not authorship/merge engine.
- **Provenance (kind 7):** canonical list of input revision IDs and source/conflict occurrence records; recipe/driver type and exact version. No output RevisionId.
- **ConflictGroup (kind 8):** deterministic fixed-fanout, sorted immutable **source alternative values** for over-wide conflicts; causal/side roles live only in Provenance. Exact normalization is in section 12.
- **PublicationCandidate (kind 9):** immutable exact target-generation/source/recipe/policy commitment; evidence attests to this ID without entering it. Its field order is in section 12.
- **Other immutable protocol, evidence and backup records:** distinct future kind tags and versioned schemas; never reuse historical tags.

The object envelope is the *hash input*, not a claim that physical storage writes one file per payload. A MapId reference carries the **actual** MapLeaf or MapBranch kind tag; either is valid only in a map-reference slot. A State root carries an Entry descriptor constrained to Directory or a Conflict whose alternatives are directories. A decoder must reject pathological allocations before allocating by an untrusted length. Different object-kind registries and future codec revisions require **new IDs**; old decoders and explicit ID mappings remain supported for histories that have been published. Hash IDs verify integrity, **not authenticity, authorization, secret protection or object availability**.

## 4. Source namespace and deterministic Merkle map

Canonical file types for v0: regular file with exact Blob bytes and executable bool; symbolic link with exact target bytes; directory, including empty; and a typed unresolved conflict. Do not canonicalize mtime, owner, OS-specific ACL/xattr, hard-link identity, device nodes, generated file contents or line endings. Unsupported admission is an explicit error, not lossy conversion.

Path components: nonempty, raw byte names, no NUL or slash, not "." or "..". v0 upper bound **255 bytes per component**, **4096 path bytes** including separators, **256 components**. A directory cannot also be a file/symlink. Reject duplicate child names. Case/Unicode normalization is **not** part of source equality; materialization on a filesystem unable to represent names must fail safely and preserve original source. Symlink target content is data; never follow an untrusted link when materializing the tree or checking containment.

**Choose a hierarchical Merkle directory tree whose each child map is one deterministic path-compressed radix map over byte-name nibbles.** The map is independent of insertion order. The particular algorithm is fixed for v0:

1. Encode each child name as a sequence of high/low four-bit nibbles. The entry descriptor is one tag plus its BlobId/MapId/ConflictId and optional executable flag. At a map trie position, keys are remaining nibble suffixes.
2. **Leaf:** if the canonical leaf representation of all remaining entries at this position fits **4096 bytes**, encode exactly one sorted leaf. Leaf payload is u32le entry count followed by records sorted lexicographically by full recovered byte name: u16le suffix nibble count, packed high-nibble-first suffix bytes (zero unused low nibble), then canonical Entry descriptor. Empty map is one empty leaf. No leaf may exceed 4096 encoded bytes.
3. **Branch:** otherwise take the **longest common remaining nibble prefix**, encode it once, then create a terminal Entry if a name ends there and child MapIds for nonempty next-nibble groups. Payload is: u16le prefix nibble count, packed prefix, u8 terminal-present, Entry only when flag=1, u16le bitmap, then child kind+hash in ascending nibble order. The branch consumes that prefix plus one nibble on each child edge. At least two terminal/child outcomes are required; one-child chains are path-compressed. Child groups recursively apply the same leaf/branch rule.
4. **Canonical normalization:** if a branch's entire key/value set would fit in a leaf, it must be a leaf. A branch's prefix must be maximal. Empty children, uncompressed one-child chains, overlapping ranges, duplicate keys, bad nibble padding, or inconsistent subtree membership are rejected. This makes identical maps yield identical encodings **regardless of editing/update order**. An update at a threshold may rewrite the affected local subtree but not unrelated sibling directories.
5. Directory entries reference a child map; relocation/reuse of an unchanged directory subtree changes only its ancestor links. A map is immutable; range scan/lookup/diff use nibble order and Merkle subtree equality. Sparse membership and absence witnesses must include the full trusted ancestor path and any relevant leaf/branch boundary proof, not just an unverified "not found" response.

**Name encoding example:** both names "a" and "ab" can coexist. An ending name is a terminal at its path; it must never be confused with a longer name sharing its prefix. Odd-length nibble prefixes carry a zero unused padding nibble and must be validated. Separate directory levels prevent confusing a filename containing slash with nested paths.

**Normative construction/check:** For one child-directory map, take its unique sorted (raw byte name, typed Entry) pairs. `Build(S)` emits a MapLeaf **iff** `4 + sum(2 + ceil(remaining_nibbles/2) + encoded_Entry_size) <= 4096`. Otherwise emit a MapBranch at the **exact** longest common nibble prefix of all names in `S`, with a terminal only if a name ends there; partition the remaining names by the next nibble, recurse with that edge consumed. For a singleton key the leaf bound always holds under v0 name/Entry sizes. Empty MapLeaf payload is exactly `00000000` in hex (u32le count zero; **four zero bytes**), and its kind tag is 2 in the enclosing hash framing; no empty branches. This definition is a function of `S`, not of insertion sequence, cached page boundaries, or edit order. A full verifier reconstructs sorted names in the **parent's nibble context**, checks complete byte names, prefix maximality, leaf-threshold inequality, bitmap/terminal cardinality, no duplicate/range overlap and child canonicality, and requires reconstruction to re-encode to identical objects. A leaf stored below an odd-depth nibble edge may have odd-length *remaining* suffixes; an ending **complete name** or terminal may not end halfway through a byte. Bare child MapIds describe suffix maps, not absolute paths: validity must be checked from a directory-map root with accumulated prefix parity and path bounds. A thin reader cannot certify an unavailable subtree merely from an outer digest; complete validation belongs to a holder of the required closure.

Why not a flat single directory blob? Wide directories would rewrite it on every small edit. Why not canonically require a content-defined POS/prolly B-tree? Ordered scans and page packing can be excellent, but source path-prefix lookup, subtree moves, and deterministic upper fanout support this radix choice with fewer boundary-shift dependencies. **Physical** cache pages/batches and optional query indexes remain free to use different layouts without changing StateIds.

The **4096-byte leaf target**, nibble branching and codec above are v0 format choices. They are not performance facts and must pass actual golden vectors, adversarial-wide-map and byte-size checks before the format becomes usable. After release, tuning physical packing must not silently change canonical hashes.

## 5. Constructing and observing source

Source construction is a pure conditional transform against an immutable base StateId:

    Apply(base_state, requested_edits, expected_path_witnesses)
      -> either CompleteStateId or typed Conflict/Error

Edits: Put(path, expected_previous_or_absent, new Entry), Remove(path, expected), CopySubtree(source typed immutable subtree value, destination, expected_destination), and MoveSubtree(source path, destination, expected_source, expected_destination). The source of a Copy is a content-addressed immutable value, **not a reference to an otherwise-dead historical revision**. A move is the corresponding source binding removal and destination insertion with value reuse, checked atomically.

Structural-phase expected path/absence conditions refer to the pinned **base**; ordered leaf-phase witnesses refer to the computed post-structural view. Structural operations are resolved from that base as one batch: verify all source and destination witnesses, acquire protection for every reused subtree, remove all move sources, then install nonoverlapping copied/moved destinations atomically in the new immutable value. A destination's existing value cannot be overwritten without an explicit exact expected value and policy. Reject source/destination aliasing, move into its own descendant, conflicting paths, ambiguous chains/swaps and structural cycles; there is no unspecified order among requested structural operations. A separately ordered leaf-edit phase may explicitly target post-structural names with exact witnesses from that computed view (e.g. move src/ → lib/ then edit lib/x). Before building the destination root, prove **all descendants** of moved/copied subtrees still satisfy max 256 components/4096 path bytes after relocation; a trusted cached subtree depth/maximum-relative-path summary may supply this proof only if previously **computed by complete traversal and validation** of the exact typed subtree hash and codec version; otherwise independently traverse the subtree. Imported unverified summaries have no authority. No claim that a subtree move is O(1) when an uncached bound check must expand it. Reject the whole transform on any failed condition; no partial StateId is admitted.

The API can preserve a **recorded user-commanded Move** with explicit source/destination identity and conditions. A filesystem observer sees deletions/creations; even a unique exact byte match proves content equivalence, **not rename intent**. Inferred matches and similarity are advisory hints and may guide a user-confirmed merge recipe, never silently decide a contested delete-vs-move or edit-vs-move. A directory move is cheap when the immutable subtree is known; an unrecorded move-and-edit may require more changed paths or a conservative conflict.

**Normal filesystem capture:** tracked/admitted files are enumerated and hashed; old immutable base provides unchanged path values. Watchers, stat checks, double scans and mtime caches are hints. Without exclusive writers or a real filesystem snapshot, even two successful scans do not prove that all captured files coexisted at one point in time. The State identifies exactly the bytes AVC admitted; its authored Revision must carry observation scope/consistency provenance instead of claiming a live-directory instant. A user-selectable partial-path author operation uses unchanged base for unspecified paths and never silently includes secrets or ignored/untracked files.

**Partial client:** a trusted service verifies the complete exact base State, all affected membership/absence conditions and rebuilt root closure, or a client holding all sibling commitments and verifiable witnesses recomputes that root locally. A proof is anchored to an authenticated *authorized* StateId, not merely a claimant-supplied digest; proof availability is distinct from permission to disclose it. Ordinary raw Merkle leaf/branch witnesses reveal neighboring names, Entry types and hashes. **Path-restricted principals are therefore not given raw ancestor proofs** unless the whole disclosed node set is authorized; their edits must be verified and rooted server-side and returned as a signed/authorized scoped result. If a disconnected client lacks sufficient authorized closure, it may durably queue a pending conditional edit bundle, **not** claim a complete validated StateId. A full local clone remains self-contained and offline.

## 6. Merge and conflict semantics

**Default:** conservative three-way source-state merge; deterministic for fixed input source values, recorded structural hints and driver version, but not associative or a proof of program correctness. The exact algorithm, content-derived work budgets and ambiguity policy are part of a merge *recipe*, not of the content-object decoder. Transient execution failure (allocation failure, deadline, cancellation, unavailable object, I/O failure) returns a typed aborted/pending outcome with **no output StateId**, never a fabricated source conflict. Preserve promised input protection and allow a retry; only explicit versioned, content-derived checks may select a conflict result. The basic value rules are:

    if L == R:                          return L
    if L == B:                          return R
    if R == B:                          return L
    if B,L,R are all directories:       merge each child with its B/L/R values
    if B is known absent and L,R directories:
                                        merge independent child additions
    if B,L,R are suitable text files:   bounded conservative line merge
    else:                               unresolved typed conflict

Directory recursion with B absent treats each child as absent; equal additions coalesce and unlike same-name additions conflict. If B was a file/symlink/conflict or an indeterminate type, two *different* new directories do **not** silently union. If a parent is deleted on one side and modified on the other, form a conflict over the whole affected subtree, never discard the modification. File mode, type, symlink and binary changes follow equality/base rules only in the default driver.

**Candidate text driver `avc.text.lines/v1`:** After equality/base rules, consider only three regular files with equal executable mode, valid UTF-8 **without NUL**, each at most 1 MiB. Tokenize as exact byte lines, retaining LF/CRLF/CR terminators; no synthetic empty line after a trailing terminator. An alignment is a chain of byte-equal `(base_index, side_index)` matches with both indices strictly increasing. Compute an exact maximum-length chain and its number of optimal pair sequences, saturated at two. Automatic combination requires **one unique optimum on each side**; ambiguity is a path conflict, not permission to choose an anchor heuristic. No prefix/suffix trimming may force an otherwise ambiguous match.

For each base-to-side comparison, first compute `r = sum(freq_base(line) * freq_side(line))` without enumerating pairs. A path conflict results if `r > 4,000,000` or `r * ceil(log2(side_line_count + 1)) > 64,000,000`; these are deterministic candidate driver budgets, not timing/performance claims. Otherwise use sparse prefix-maximum dynamic programming over match pairs. Each pair's state counts maximum chains **ending at that pair**; query only strictly smaller side indices and defer all updates for one base row until its queries finish. Aggregate disjoint endpoint ranges by longest length and saturated sum of counts; the empty alignment is exactly one chain. Fenwick or equivalent prefix indexes avoid the old full-file quadratic cell requirement. Reconstruct anchors only for a unique optimum; a diagnostic tie-break must not change source output.

Derive half-open replacement intervals and zero-width insertion gaps from the anchors. Combine disjoint replacements and insertions at distinct gaps **outside the closed boundaries** of another side's replacement; identical hunks coalesce once. Thus a replacement `[3,5)` conflicts with another side's insertion at gap 3 **or 5**, intentionally conservatively. Different insertions at one gap and any other overlap become a path conflict. Scan sorted hunk lists rather than comparing every pair. Emit exact unchanged base spans and side bytes in base order, without terminator normalization. Execution failures follow the no-State rule above. The [text reference](experiments/text_merge_reference.py) checks sparse alignment against exhaustive small pair sequences and exercises distant edits in a 2001-line file, repeated-line ambiguity, boundaries and CRLF. These proposed budgets still need representative workflow qualification before driver freeze; after release a rule/budget change needs a new driver version, not a new object decoder.

**Recorded-Move merge rule:** Use an explicit side's Move(s→d) as a path mapping only if the original base `s` has the exact old Entry, the mapping is one-to-one and prefix-injective, the side's resulting destination is exactly the moved subtree **modulo separately recorded checked edits**, and the other side has not structurally moved/deleted that same source or changed/occupied `d`. Transport ordinary modifications/deletions to **already existing base descendants** of `s` under `d` and apply them with the same three-way equality/text rules against the unchanged base values. Edits that are new additions under an old directory name `s` remain **coupled conflicts by default**: intent to follow the moved directory cannot be deduced. A source-only merge has no move intent. A history merge must explicitly supply validated move evidence relative to its selected base and each side; it must not search arbitrary ancestry and guess which old commands still apply. The evidence binds exact base/side StateIds, ordered checked structural operations and post-structural edits, and must replay to that side's State before it can authorize remapping. A move recorded further back or on a rebased successor is usable only through a newly checked base-to-side recipe. Ephemeral virtual revisions carry no inferred move intent; any supplied evidence must be checked against their exact computed States. Two verified sides moving the same source to the same destination can merge the descendant edits; different destinations, transitive/colliding mappings, new conflicting paths, delete-vs-move and ancestor/type collisions produce a conflict at the least common ancestor containing every coupled original/destination path. Do not allow a proposed mapping to overwrite another side's files, drop a deletion/edit or silently modify unaffected paths. Inferred content matches/similarity from ordinary filesystem scans are **suggestions requiring user-confirmed recipe**, never automatic evidence of identity.

**Merge bases:** For two Revision DAG tips, compute *all* maximal common ancestors (no common ancestor is itself an ancestor of another). With one base, merge its State. With none, refuse by default; an **explicit unrelated-history merge** uses the canonical empty-directory State as synthetic base and records that choice. With several, reduce their full sorted RevisionIds by recursively forming ephemeral virtual revisions: `V1=B1`, `Vi=MergeRevision(Vi-1,Bi)` with parents `[Vi-1,Bi]` and its computed State. An ephemeral virtual node is **not authored, persisted or given a RevisionId**; it carries explicit parent ancestry and a deterministic recipe key for memoization. To merge each pair, compute its own maximal common ancestors in this augmented finite DAG (strictly earlier than both inputs), recursively reduce that frontier, then merge the two source States over the reduced base. Thus later pair merges retain the ancestry of earlier virtual nodes rather than incorrectly merging two source snapshots without a base. The versioned recipe records original frontier, reduction order, each virtual node's inputs and computed State, drivers and any explicit synthetic-empty-base choices. A recursive pair with no common ancestor also requires explicit unrelated-history permission; two tips already sharing unrelated roots do not authorize silent synthesis during frontier reduction. Detect graph cycles, bound work/depth and return `MergeTooComplex` **without admitting a resulting State** if bounds are hit; retain/pin input States and a pending merge recipe instead. Source conflict *width* groups (section 12) cannot magically represent an uncomputed/deep virtual merge. N-way proposal composition is explicitly ordered, not claimed associative or commutative.

### Conflicts are typed source values, not marker files

    ConflictValue {
      base: EntryValue,                      // MaybeEntry, absence is real
      alternatives: Inline(2..64 sorted distinct EntryValues)
                  | Group(>64 sorted distinct values),
      kind: PathValue | CoupledSubtree
    }

The canonical ConflictValue contains **only unresolved source semantics**; source EntryValue can be absent, File, Symlink, Directory or an earlier ConflictValue reference. The outer StateId includes the conflict value; equal unresolved semantic values have equal source hashes. Source conflicts can be file, path, directory, binary, delete/edit, and **coupled across multiple paths** (e.g. divergent renames).

For a multi-path coupled conflict, place an Entry::Conflict at the **lowest common ancestor subtree** containing *all affected bindings*, using complete immutable directory alternatives from the selected sides/base as necessary. At the State root, only **present recursively directory-valued** alternatives are legal, including within nested conflicts—a deletion hidden below another CoupledSubtree is invalid. Reads of a descendant traverse **every** alternative (including nested conflicts); if all alternatives yield the same typed Entry/absence, return it as agreed, otherwise return an unresolved **query result**, not a fabricated persistent Entry. A non-directory encountered on one branch cannot be silently interpreted as a directory with absent descendants. Missing/promised objects yield Unknown, not absence; no incomplete answer proves agreement. Materialization never silently chooses a branch. This permits agreed siblings under a high-level conflict without changing the canonical State.

Normalization rule: equality/base simplification occurs **during a particular merge operation**; it is not a decoder license to invent missing history. Otherwise sort alternatives by canonical encoded semantic EntryValue, deduplicate exact terms, preserve the (possibly nested conflicted) base and source alternatives, and resolve to one Entry only when justified. Do **not** flatten arbitrary nested conflicts or assume identical bytes imply identical causal edits. A kind-8 ConflictGroup is the **canonical width representation only** for more than 64 sorted distinct alternative values, not an escape hatch for excessive graph depth, unresolved merge-base computation or unbounded expansion. On nesting depth >64 or other resource budget failure, return `MergeTooComplex` without a new authored State; durably retain original input roots/pending proposal under an explicit retention receipt if requested. No acknowledged result may discard alternatives or claim to be a conflict State it cannot decode.

**ConflictOrigin/Provenance** is *separate*: input RevisionIds, relevant paths and side roles, base frontier, exact driver/recipe version, move hints, and recorded resolution choices. The output Revision owns a ProvenanceId; the provenance never names that output RevisionId (no reference cycle). Two revisions may contain an equal conflict source value with different causal explanations. v0 is not a causal patch algebra: deduplicating identical source terms is a **source equality choice**, not a claim of causally correct selective undo.

**Presentation:** conflict marker files are optional, generated lossily for an ordinary editor. The Materialization authority owns a manifest binding destination identity, selection/materialization generations, path, exact generated marker bytes/hash, source ConflictId and presentation codec. Persist that manifest with the protected journal **before** marker bytes can become visible. An unchanged bound marker remains the original Conflict Entry on capture/import. Changed marker bytes are preserved as **pending resolution observations**, not admitted File replacements; promotion/import requires an explicit validated resolution. If a path crosses a source conflict but the manifest is missing, stale or incomplete, fail closed: preserve observed bytes and uncertainty without guessing that the conflict was resolved. Never infer conflict identity by parsing marker-like text in an ordinary file. Binary/directory/coupled conflicts may have no marker presentation. Resolve by choosing or constructing new Entries; prior conflicted inputs remain inspectable.

## 7. Rebase, amend, undo, revert and change identity

**Rebase** of single-parent Revision C with old parent P onto new base Q is a defined three-way source transformation:

    outcome = Merge(base=State(P), left=State(Q), right=State(C))
    C' = new Revision(state=outcome.State, parents=[Q],
                      logical_change=C.logical_change, ...)
    Evolution(C -> C', Rebase)

The result may be conflicted; old C remains immutable. An explicit selected stack is rebased in topological order, mapping already-rebased parents. Do not automatically rewrite unrelated descendants or accepted targets. Rebase of a multi-parent merge requires an explicit parent-mapping/merge recipe; v0 **rejects unsupported merge-commit rebase** rather than guessing.

**Amend** constructs a replacement Revision from an explicitly chosen new State/metadata with the **same ordered parents and ChangeId**; it does not mutate the old Revision or implicitly rewrite its descendants. **Undo** conditionally restores scoped private root selections/capture/operation outcomes, with preserved receipts; it does not overwrite live filesystem bytes. For single-parent revision R with parent P, **Revert(R onto Q)** is `Merge(base=State(R), left=State(Q), right=State(P))`; **Reapply(R onto Q)** is `Merge(base=State(P), left=State(Q), right=State(R))`. Both may conflict and create newly authored revisions rather than resurrecting old RevisionIds. Reverting a merge revision requires an explicit selected mainline parent, recorded with the recipe; a root revision has no implicit inverse. No everlasting "seen ChangeId" blacklist; divergent logical ChangeId successors remain distinct immutable revisions.

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
      base_StateId, optional_exact_base_RevisionId, observed_selection_generation,
      admitted_bytes_and_deletions, pending_resolution_observations,
      covered/failed/skipped_paths,
      consistency: PerFileObserved | WriterQuiesced | FilesystemSnapshot,
      durable_receipt, created_at, retention_deadline
    }

Capture may use immutable overlays/deltas against a pinned base with bounded replay depth. It is not an authored Revision and does not trigger public share, CI, review or target movement. Only bytes actually admitted and acknowledged are protected. Capture triggers: explicit request or explicitly configured editor/agent boundary; a watcher/daemon is optional and must report missed-event gaps. A capture's original base, any recorded base Revision and all admitted/pending bytes stay pinned through its promised retention. Promotion cannot release its original receipt's protection early unless the user explicitly changes that retention obligation. If a concurrent selection change makes a capture outdated, keep its acknowledged bytes as historical recovery rather than retarget it. The capture head can fail to advance on CAS, while the separately acknowledged capture remains pinned. Distinct selection and capture generations eliminate false logical conflicts.

**PromoteCapture:** Author the exact capture value against its recorded base, not the directory's current bytes or current selection. Uncovered paths keep base values; failed/skipped coverage requires explicit confirmation of that partial scope. Pending marker resolutions require an explicit checked resolution recipe. Validate complete resulting source closure before admission. Default history parent is the exact recorded base Revision (whose State must equal the capture base); never infer a parent by looking up equal StateIds. A State-only capture requires an explicitly chosen matching parent or explicit zero-parent authoring. Promotion creates a fresh ChangeId and caller-supplied immutable author metadata, stores observation provenance, and transfers/establishes authored-root custody in the normal idempotent admission transaction. It does not silently advance Selection or CaptureHead: an explicitly requested pointer change needs that resource's generation check. A lost response retries the same receipt/ChangeId, not another revision; later separately requested authoring may reuse the immutable capture. Original capture retention and authored revision retention remain distinct.

**Observation provenance:** Authored revisions from filesystem observation or capture promotion must carry a known versioned observation recipe in Provenance, even when no merge/move occurred. It records the base identity, observation scope/consistency, covered/failed/skipped paths, capture receipt identity if any and explicit partial/resolution choices. This label is history metadata, never State identity; no label claims continuing filesystem cleanliness. The recipe's capture/receipt identifiers are non-owning audit identifiers and do not promise that expired recovery bytes remain available. Native export retains this provenance with the revision.

**Default retention:** private captures seven days unless explicitly changed; unpinned orphaned authored revisions ninety days; accepted or explicitly pinned roots while selected/referenced, subject to deliberate retention policy and backup. These are v0 defaults, not claims of infinite durability. Quota exhaustion must reject new captures clearly, not delete older still-acknowledged captures without respecting their horizon.

**Two materialization contracts, for usability and safety:**

- **Fresh-directory restore / new workspace** is the default for recovery and agent branching, and the only portable strongest no-clobber path. Allocate a new empty destination, record durable intent, construct files with dirfd-relative safe operations, verify them and mark ready. Never overwrite the original working directory.
- **Ordinary in-place switch** is supported for editor ergonomics, not atomic against arbitrary writers. Serialize **materialization operations for the same destination identity**, independently of other workspaces. Before any mutation, scan all affected tracked/untracked paths and nested obstacles; refuse dirt, coverage gaps, unknown obstructions and case collisions by default (including untracked files inside a directory slated for removal). Require explicit preservation policy or managed writer exclusion to proceed in those cases. Admit and **durably pin a before-image of every path at risk**, then persist a journal binding destination identity, selection/materialization generations, intended State, path-level pending actions, marker-presentation manifest and recovery receipt **before** changing the first path. Stage files outside conflicting names, perform path-safe guarded replacements/removals with recorded per-path outcomes, then verify from disk and conditionally finish only against the same selection/operation generation. An uncoordinated editor may race even between verification and return; `Verified` means **verified at recorded observation**, never a continuing clean guarantee. After a crash, journal replay may finish/restore only if inode/path guards still match; otherwise leave `Incomplete`, preserve before-images and require deliberate recovery in a fresh directory. No blind replay or rollback over newer third-party edits. Managed writer exclusion (or actual OS snapshot plus coordination) is needed for stronger guarantees.

OS check-then-rename does not revoke open descriptors. Use dirfd/handle-relative traversal without following untrusted symlinks, cross-device/unsupported-operation checks, flushes appropriate to the declared crash guarantee and destination identity rechecks. Materialization may use CoW/reflinks/overlay/VM snapshots, but none is mandatory. A crash leaves pinned before-images plus an explicit incomplete outcome, never a fictitious complete checkout. A capture records admitted bytes per path and explicit failed/skipped coverage; a quiesced or snapshot consistency label requires evidence of the claimed writer/FS boundary, not just two matching scans.

**Optional managed-workspace boundary (not a v0 prerequisite):** A future mounted/virtual filesystem may observe operations and use copy-on-write private overlays, stable *workspace-local* inode/handle identities and kernel cache invalidation to avoid ordinary post-hoc scans. These IDs and incidental OS metadata remain outside canonical StateId; recorded rename intent requires verified original/destination bindings, not an assumption that equal contents imply a move. `WriterQuiesced` or `FilesystemSnapshot` requires proof that **all relevant writers, including buffered/mapped I/O and outstanding handles**, were covered or stopped at the stated boundary; an intercepted write alone does not prove a coherent snapshot. Distinguish write visibility, OS-level `fsync`/`fdatasync`/directory-sync completion, protected Capture receipt and fully durable Revision/draft-root acknowledgement. A mount advertised as durable must not report successful sync when backing persistence is unimplemented (including an `ENOSYS` FUSE response the kernel treats as success). Failure or uncertain mount teardown must retain recoverable acknowledged source; invalidation of open handles/dirty pages on root switch must be qualified per platform. No FUSE, daemon, container, or virtual mount is required for ordinary AVC workspaces.

## 9. Durable storage, crash ordering, GC and backups

Local authority is **SQLite WAL with synchronous=FULL for acknowledged commits** on qualified local filesystems. It owns short transactions, small immutable objects, all mutable roots/generations, protection leases/pins, object admission and reclamation states, receipts and an event outbox. WAL is not a shared multi-host distributed database. External large immutable objects occupy immutable content-addressed paths. The critical cross-store invariant is: **no durably committed acknowledged root may reference missing, unprotected or half-published object closure**.

**Admission protocol (required order):**

1. Acquire a durable staging record and **GC-safe protection of the entire source dependency closure**, including reused base/copy input roots and preexisting unrooted objects, *before* relying on them. Root pins can protect reachable subtrees; newly admitted object IDs need stage leases. If a required object is already marked for reclamation, abort or restage and retry rather than racing its unlink. A reserve alone does **not** prove the bytes are present.
2. Verify the logical payload, kind, canonical decoding and all references. Write every new external object to a unique temp path, flush file contents and required metadata, atomically install without overwriting a different value, **fsync the containing directory** on platforms where that is the persistence contract, and verify final object accessibility. Propagate failures; never acknowledge solely on hash calculation, rename or buffered write.
3. In **one short FULL SQLite transaction**, first resolve idempotency (same scoped key+request digest returns its prior durable outcome; a changed request under the same key is rejected), then validate expected resource generations, authorization/policy epoch and **complete protected durable closure**. Transfer staging protection into the new retained root/pin, persist the root, receipt and outbox event atomically. A failed check changes none of those logical resources. A successful commit is the acknowledgment boundary; a reply loss does not re-execute side effects.
4. After a crash, reconcile staging and external files: old committed roots remain reachable; interrupted temp installs are either verified/reused or discarded only under the GC barrier. Reopen/validate receipts and roots before returning results. The file system and SQLite log must share a supported ordering and failure domain; never claim crash atomicity across unrelated volume/host failure domains.

**Reader/GC interlock:** A reader takes a protected root/closure lease under the same transactional authority **before** its first object dereference and releases it after use. A mark phase over a root snapshot is **not enough**: an old, previously unmarked object may be re-pinned by a concurrent author. Reclamation must first atomically mark each candidate `Deleting` (a durable tombstone) in the SQLite authority after checking **current** roots, staging, reader and backup/export leases. New pins/admissions must transactionally reject `Deleting` objects; existing leases prevent tombstoning. Only then may the collector unlink external files, retire small-object records and later clear tombstones; physical object-path reuse must not race an outstanding unlink. Crash recovery completes/rolls back tombstone cleanup without making removed objects appear resident. Compaction uses the same lease/fence discipline. GC traverses all retained Revision/State/Conflict/Provenance closure, captures, drafts, target roots and protected backup roots—not just immediate blobs.

**Backup/export barrier:** Atomically read the exact root set and establish its **durable backup lease within the same short write transaction** (e.g. acquire SQLite `BEGIN IMMEDIATE` before that root read, avoiding read-snapshot promotion races). Commit the lease before releasing the selected root snapshot and enumerating external objects. The lease owns an immutable descriptor of the exact selected root set and generations, even if a live root later advances or expires. Export that snapshot's complete typed closure and a manifest recording root generations, retention and codecs; fsync and verify both closure and equality with the leased root-set descriptor before dropping the lease. A disappearing live root must not silently shrink the export. An SQLite online-backup snapshot alone is insufficient because GC could delete its referenced external objects during the copy. Raw DB backups must include corresponding root-set protection/manifest at the same logical snapshot; independent native exports need not reuse SQLite's physical layout. Corruption checks and a reconstruct-on-another-machine restore test are required.

These ordering rules rely on honest sync, atomic-install and locking semantics of a **qualified** local filesystem/host; they do not survive lying hardware or loss of every acknowledged replica. Remote receipts must separately state the chosen server replication/durability class. Hashes detect corruption, not repair it. Retention expiry and explicit deletion are outside the promised retention horizon; loss before that horizon is a correctness failure.

**Physical layout choice:** initially whole logical blobs, small SQLite object records and large immutable files. Later byte compression, content-defined chunking for huge edited assets, bounded delta chains, packs, cache pages and multi-region tiering may optimize access without altering BlobId/StateId. An all-in-SQLite object placement may be compared as a **physical alternative**, not imported as a new source model; any selection must match acknowledged WAL/FULL durability, reader/GC pins, backup closure and large-file behavior, rather than borrowing measurements from a disposable SQLite profile. Do not canonically require Enroute's indexed layers, Git packfiles, global dense IDs, Raft or S3. Physical optimization is evaluated under identical durability, cold/hot reads, GC cost and transfer patterns.

## 10. Native protocol, hosting and publication

One source-operation contract works as a local library and a networked service. Operations include typed GetState/GetPath/GetFileRange, ListDirectory, DiffState, ancestry/history/blame query (derived), conditional Apply, CreateRevision, Merge, FetchObjectBatch, native bundle exchange, PinDraft and target proposal/acceptance. Requests have negotiated format/protocol family, bounded lengths/recursion/transfer budgets and explicit complete/pending/unknown outcomes. Mutations carry a caller-scoped idempotency key, canonical request digest and expected generation/epoch; receipts persist failures whose outcome must not be repeated, successful roots and associated event IDs. Requests are authenticated and independently authorized at resource/closure scope; do not imply that a hash grants read permission.

**Full client:** holds the object closure for selected retention roots, can author/inspect/merge offline. **Partial client:** explicitly records promised missing objects, hydrates by exact ID/path witnesses or asks service to build verified source. **Thin remote client:** may query/compose without any working tree checkout. **Bulk transfer:** compressed, resumable selected-object closure/history bundles avoid one network RTT per node; sparse exact-object queries avoid shipping whole history. No native Git format requirements.

**Git conversion boundary (deferred):** A converter declares its source/history subset and an explicit native-to-Git ID mapping; Git commit/object IDs never substitute for native State/Revision identities or attestations. Git has no native typed-conflict value, ordinary Git index/checkout workflows do not preserve tracked empty directories, and v0 cannot directly represent Git submodule/gitlink entries. Metadata and names may also be unrepresentable. Reject unsupported/lossy conversion by default; any user-authorized loss needs a manifest and must not be advertised as a lossless round trip. An opaque native archive carried alongside Git is archival preservation, not an ordinary Git source representation. Native verified export remains the full-fidelity custody path.

**Ephemeral agent invariant:** the worker may report safe termination **only** after a receiving authority durably acknowledges a **pinned immutable Revision/State draft root and its complete authorized closure**, with an operation receipt, retention deadline/durability class and ability to fetch/reconstruct it. Resumable uploads and retries do not turn incomplete/promised objects into an acknowledged draft. After an unknown response, query the exact idempotency receipt; do not destroy the sole surviving source until a successful durable receipt is verified. Worker runtime/VM state, toolchain cache and generated binaries are not source history.

**Publication candidate** is immutable and binds target identity, **monotonically increasing observed generation** (not only the observed root—avoid ABA), observed target RevisionId, ordered chosen RevisionIds, exact deterministic merge recipe/driver versions, computed resulting StateId and RevisionId, and validation-policy digest/epoch. The service recomputes/verifies that the named revision/source and recipe are consistent; for an explicitly manual result the recipe contains checked source operations and the service verifies their output against the submitted State. A client assertion is not a trusted proof of integration. Validation evidence is a separate, authenticated immutable attestation **over this exact candidate ID** plus input sources, build/test/toolchain versions, declared environment, status and policy identity; it cannot enter the candidate ID (no cycle). An attestation does not establish correctness outside its declared inputs/assumptions. Validation runs outside target transactions.

**Acceptance:** In one durable linearizable transaction for **that target only**, resolve the idempotency key and immutable prior receipt first, then check candidate's expected target generation/RevisionId, policy epoch/digest, authorization at the decision point, evidence freshness/trust under current policy, complete pinned source closure and required history policy. A protected linear-history target ordinarily requires its current RevisionId to be an ancestor of the submitted revision (unless an explicit non-fast-forward policy permits otherwise). Only then CAS the target root and increment its never-reused generation; atomically make the accepted Revision's complete closure target-owned retention, and commit acceptance receipt and durable outbox event together. If object custody is in a separate authority, establish its durable target-owned protection before the CAS and bind its receipt to the acceptance; a temporary upload/proposal lease is insufficient. Retain that protection while the accepted root or another promised retention obligation needs it; an unknown CAS outcome is not permission to release it. The caller may safely retry unknown outcomes with the same key+payload; a conflicting payload under the key fails, and consumers dedupe at-least-once outbox delivery by event ID. Authorization revocation can limit receipt disclosure without undoing a prior committed acceptance. On stale target/policy/evidence, retain the candidate and source as a proposal; rebuild and revalidate a **new candidate** rather than transplanting old evidence.

Default protected target: **validate before acceptance**. An explicit high-throughput policy may separate accepted and verified target roots, with deployment gated on verified. A clean text merge is never sufficient evidence of program correctness. Long-lived deployment artifacts carry the exact source RevisionId, build recipe and validation provenance; production processes need not host AVC.

**Security and multi-tenancy:** knowing a content hash does not authorize reading it. Enforce per-principal rooted object-closure and path policies **before** object fetch, existence testing, batch enumeration, diff, CAS diagnostics, proofs and cache responses; use uniform unauthorized-error behavior. A canonical unkeyed digest inherently reveals equality **to parties already holding both IDs**, so do not promise cryptographic secrecy from hash equality. Raw Merkle witnesses can expose neighbor names and hashes; serve filtered path-scoped answers without raw cross-policy sibling nodes (or explicitly authorize such disclosure). Untrusted content never implicitly executes hooks/merge programs. Authorship/signatures are authenticated attestations outside hash integrity. Ignore patterns cannot expunge secrets from retained immutable history/backup.

## 11. Adversarial invariants and decisions

| Failure/edge case | Mandatory behavior |
| --- | --- |
| Equal source from different edit orders/histories | Identical StateId under v0 codec, possibly different RevisionIds |
| Tiny edit under a million names | Update affected Merkle map path only; no unbounded whole-directory rewrite |
| Directory rename + edit, or copy from old branch | Content subtree reuse with pinned input closure and proven relocated descendant path bounds; no permanent old-revision dependency |
| Two names differ only by case on case-insensitive OS | Source retained; unsafe materialization fails without silent rename |
| Two sides rename one file differently | Coupled subtree conflict, preserve both destinations and original values |
| Criss-cross merge/previously conflicted base | Augmented ephemeral virtual ancestry; bounded algorithms either retain all representable conflict terms or return MergeTooComplex with inputs protected |
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

**Qualification before format freeze:** exact golden vectors and parser limits, conflict normalization and text-merge fixtures, filesystem fsync/name portability, and representative resource/layout comparisons. Correct or replace a failing candidate before stable user data exists; after release, changing canonical bytes requires a new format identity and explicit migration. Evidence may revise a selected implementation without reopening product requirements.

**Current implementation:** none of this native engine is implemented. Rust 0.0.1 remains a Git-command/JSON-log prototype with none of the advertised native guarantees. The research fixtures are not release qualification.


## 12. Normative v0 field order and canonical expansion rules

This section fixes the **object payload grammar** referred to in sections 3–6. Notation: byte[n] is exactly n bytes, ref(k) is u8(actual kind) plus byte[32] digest with kind validation, and str is u32le byte-length plus raw bytes. Hash envelope is defined in section 3 and is not repeated inside a payload.

    Entry =
        0x01 ref(Blob) u8(executable=0|1)      // ordinary file
      | 0x02 ref(Blob)                         // symlink target bytes
      | 0x03 ref(MapLeaf|MapBranch)           // directory
      | 0x04 ref(ConflictValue)               // unresolved source
    MaybeEntry = 0x00 | 0x01 Entry            // 0=absent; 1=present
    StatePayload = Entry                      // only Directory or a
                                              // CoupledSubtree conflict with
                                              // present directory alternatives

    MapLeafPayload =
      u32le(n)
      repeat n: u16le(suffix_nibbles), packed_nibbles(suffix), Entry

    MapBranchPayload =
      u16le(common_prefix_nibbles), packed_nibbles(prefix),
      u8(terminal_present=0|1), if 1: Entry,
      u16le(child_bitmap),
      refs(MapLeaf|MapBranch) in ascending nibble-slot order

    RevisionPayload =
      ref(State), u32le(parent_count),
      repeat parent_count: ref(Revision) in authored first-parent order,
      byte[16](ChangeId),
      str(author_bytes), str(description_bytes), i64le(unix_ns),
      u8(provenance_present=0|1), if 1: ref(Provenance)

The MapLeaf and MapBranch **actual kind tags** are distinct in their object IDs. A MapRef uses the appropriate actual tag. Every decoded map is checked against canonical leaf size, maximal shared prefixes, child-map membership, sorted key uniqueness, nibble padding and forced leaf/branch normalization; otherwise it is rejected even if its outer digest verifies. An `if 1` field following its own explicit presence flag contains **only the value bytes**, never a duplicate presence byte. An Entry at a map terminal names the exact **byte-complete** key ending at that nibble prefix; a terminal at odd total nibble depth is rejected. A map object is validated in its enclosing directory-root context, including depth parity and reconstructed name bounds; an isolated child hash cannot establish a complete State witness.

ConflictValue has two storage cases:

    ConflictValuePayload =
      u8(region_kind=0 PathValue | 1 CoupledSubtree),
      MaybeEntry(base),
      u8(alt_storage=0 Inline | 1 Group),
      if Inline: u32le(n: 2..64), repeat n: EntryValue
      if Group: ref(ConflictGroup)

Here **EntryValue** is MaybeEntry, so deletion/absence can be an alternative. For Inline alternatives, sort strictly by canonical encoded EntryValue bytes and deduplicate exact equal semantic terms first; a one-alternative result must be represented as a resolved Entry, not a conflict. A RegionKind CoupledSubtree requires every **present** base or alternative to be a Directory or a **strictly directory-valued nested CoupledSubtree Conflict**. Strictly directory-valued means all alternatives recursively contain only present directories or strictly directory-valued conflicts; any absent, file, symlink or PathValue term disqualifies it. A non-root CoupledSubtree may itself directly include an absent alternative (directory deletion), but this maybe-deleted value **cannot** be embedded as a present directory-valued alternative of another CoupledSubtree. At a State root all alternatives must be present and strictly directory-valued, including nested levels; an absent base is allowed. Decoders must check all these conditions recursively through inline/grouped storage, not just the nested conflict's region tag. Equal base/side normalization and trivially equivalent outcomes apply *before* persisting the conflict. Historical side order is in Provenance, not in ConflictValue.

For more than 64 alternatives, use **ConflictGroup (kind 8)** instead of the inline list:

    ConflictGroupLeafPayload =
      0x00, u32le(n: 1..64), sorted distinct EntryValues

    ConflictGroupBranchPayload =
      0x01, u32le(n: 2..64),
      repeat n: ref(ConflictGroup) in sorted minimum-child-value order

Canonical group construction: sort/deduplicate all alternative semantic EntryValues, form consecutive blocks of **64** as leaves (including the final shorter block), then group consecutive up-to-64 child refs by their covered key ranges until one root. Leaf values and child ranges cannot overlap. At every child-reference level above the leaves, a final block with one child carries that child forward unchanged, rather than encoding a one-child branch; therefore siblings may have different heights. A ConflictValue MUST use Inline for 2–64 distinct values and Group for >64. A single nested ConflictValue is one opaque **semantic value term** for sorting, not an invitation to flatten/cancel it. Kind 8 groups encode **width only**: nesting beyond 64 or insufficient computation budgets reject construction of a new State; its inputs remain protected as separate durable roots/pending recipe. For a **non-root** `CoupledSubtree`, an absent **direct** alternative represents deletion and is allowed. Each present alternative and present base must be a directory or a **strictly directory-valued nested CoupledSubtree**: every nested alternative must be present and recursively directory-valued. A nested maybe-deleted conflict is not a directory-valued Entry. At the **State root**, `region_kind=CoupledSubtree` and **all alternatives at every nested level** must be present directories or strictly directory-valued conflicts; the outer or nested base may be absent.

**PublicationCandidatePayload (kind 9)** is an immutable transaction proposal, outside the source/revision dependency graph:

    str(target_authority_namespace), str(target_name),
    u64le(expected_generation),
    u8(observed_revision_present=0|1), if 1: ref(Revision),
    u32le(selected_count: 1..64), selected ref(Revision) in recipe order,
    str(recipe_driver_id), u32le(recipe_driver_version),
    str(recipe_bytes),
    ref(State)(result), ref(Revision)(result),
    byte[32](policy_digest), u64le(policy_epoch)

The result Revision must name the result State and satisfy the candidate's policy-checked history/recipe relationship; reject duplicate selected revisions and noncanonical target identifiers. The selected array is **ordered**, not lexicographically sorted. A target not yet created uses `expected_generation=0` with absent observed revision; an existing target uses a strictly increasing nonzero generation and present observed revision. Namespaces and target names are opaque canonical byte strings with configured length and authorization limits (not source filesystem paths). The policy digest is authenticated by the target service over its canonical policy bytes; it is not interpreted as an ObjectID. Evidence/attestation payloads carry `ref(PublicationCandidate)` plus exact validator identity, signed input/toolchain/environment/result manifests and freshness; they are **independent immutable protocol records**, never a field of the Candidate or Revision. Authentication algorithms and on-wire evidence framing must be fixed before native publication is implemented; target acceptance cannot assume unsigned or generic hashes are trusted validation.

**ProvenancePayload** is a canonical versioned *history explanation*, deliberately separate from source values:

    u32le(input_revision_count)
    repeat: ref(Revision) in algorithm-role order
    u32le(occurrence_count)
    repeat sorted by (raw path components, ConflictId):
      encoded_path, ref(ConflictValue), u32le(input_role_count),
      repeat input_role_count: u32le(input_revision_index)
    str(merge_driver_id), u32le(merge_driver_version),
    str(recipe_bytes)

An encoded_path is u32le component count followed by each component as str (zero components denotes the root); validate v0 path bounds. Provenance occurrences are strictly increasing and unique by (path components, ConflictId): compare raw component bytes lexicographically, a shorter equal-prefix component/path first, then the full typed ConflictId bytes. Length prefixes and component counts are framing, not sort keys; input indices are range checked, and their ordered positions carry recipe roles, not unordered set membership. Input revision sequence follows the specified recipe-role order and is not silently sorted/deduplicated. Provenance names only existing input revisions; the owning output Revision is the immutable object referencing ProvenanceId, never embedded within it. Unknown driver IDs/version metadata may be inspected but never auto-executed. For nonmerge revisions, provenance is required for observation/capture labels or any recorded structural evidence used by a later merge; otherwise it may be absent.

**Executable recipe gate:** The recipe driver/version identifies a data parser, not executable downloaded code. Before a driver can affect merge, promotion or publication, fix and cross-check its canonical recipe schema: exact input/base/side identities; structural operations and expected values; ordered leaf edits; merge frontier/virtual-node trace and unrelated-base choices where relevant; observation/partial/resolution scope where relevant. Reject unknown or noncanonical recipes for execution, while allowing their bytes to be inspected as historical metadata. Canonical object references own closure; audit identifiers inside recipe bytes are non-owning. Any extra objects needed to execute or verify a recipe must have explicit input/staging pins until a durable result or refusal receipt is established. Exact recipe wire schemas are a pre-implementation gate, not supplied by an arbitrary opaque string.

**Canonical size limits:** component/path bounds from section 4; author ≤4096 bytes; description ≤1 MiB; merge recipe ≤1 MiB; a provenance occurrence list ≤1 million entries, streamed and resource-limited; parent revisions ≤64; source conflict nesting depth ≤64 (count ConflictValue objects along each source traversal path, including bases and alternatives; maps, directory Entries and ConflictGroups carry the current count without resetting or incrementing it). If a complete source cannot satisfy the bound, **fail admission** with typed `MergeTooComplex` and optionally retain a pinned pending proposal; do not store an invalid deeper object by calling it opaque. All resource and overflow checks precede allocations, and all source objects are validated with cycle/closure checks before acknowledging a root. Reference parsers additionally limit open object traversals to 256 and total reference visits to 30,000 to refuse adversarial/deep graphs before host-stack exhaustion. These are explicit fixture resource budgets, not additional canonical path/history constraints or proof that a refused graph is malformed; native verifiers must qualify bounded/iterative traversal separately.

**Pinned candidate v0 vectors (2026-10-16):** The [fixture manifest](experiments/codec_vectors.json) records kind, full typed ObjectID, canonical payload bytes for small objects and size plus SHA-256/preview for large objects. The [reference builder](experiments/codec_reference.py) and [independently written strict decoder/alternate radix encoder](experiments/codec_verify.py) agree on the present fixtures, backed by a minimal BLAKE3 implementation checked against the official published hash vectors. In particular, the empty MapLeaf payload is `00000000`, with ID `avc1-b3-k02-3jc2jl6s7rrrbhdwslqnijidn7z73ly43l7yzcpk37rilrouqvoa`; the corresponding empty State payload is `03 02 da45a4afd2fc63109c7692e0d425036ff3fdaf1cdaff8c89eadfe285c5d4855c` in hex, with ID `avc1-b3-k04-vmnwhh4bsvdgr3igkr7ku35zbdkwkuwugwansvrb6vo3wniyjhmq`. The 4095- and 4096-byte maps stay leaves; the 4097-byte candidate becomes a branch.

**Format-release gate remains open:** These are executable **reference** fixtures, not a second production-language codec or implementation-independent certification. Two independent complete encoders/decoders must agree byte-for-byte and hash-for-hash on empty source, files, binary bytes, executable flag, symlinks, prefix names, leaf/branch boundaries, moves, equal states/different authors, path/root conflicts, grouped overflow, provenance, candidate binding and negative cases. The format must not be treated as stable before independent cross-implementation review.
