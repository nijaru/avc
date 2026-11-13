# AVC v1 Python reference

An independent, deliberately nonincremental encoder and full-closure checker for
[`docs/format.md`](../docs/format.md), kinds `01`–`05`, the 32-entry leaf rule,
and lowercase hexadecimal IDs. No obsolete native implementation or prior
vectors were used. This is a conformance oracle, not a source store, production
merge driver, or revision-authoring tool.

Run from the checkout root with [uv](https://docs.astral.sh/uv/):

```sh
uv run reference/check.py --vectors
uv run reference/check.py --verify reference/vectors.json
uv run reference/generate.py  # deliberately regenerate pins after format changes
```

`--vectors` checks the pinned file, its rejection corpus, deterministic
regeneration, and the `unittest` suite. `--verify FILE` checks a corpus from any
implementation; it does not regenerate expected bytes or need Rust. Both exit
nonzero on failure. BLAKE3 is the only third-party Python dependency and is
pinned in the scripts' PEP 723 headers. The scripts select Python 3.11–3.14:
the pinned binding's PyO3 version does not support Python 3.15. CI explicitly
selects 3.14 rather than inheriting the runner's interpreter.

## Interoperability JSON

The required shape is:

```json
{
  "objects": [{"id": "avc1-k02-<64 lowercase hex digits>", "payload": "00"}],
  "roots": ["avc1-k02-<64 lowercase hex digits>"]
}
```

The placeholder above is not an actual ID; `vectors.json` contains complete
examples. `payload` is the canonical payload only, not the kind, hash domain, or
wire reference. Empty Blob payload is `""`. Both ID and payload hex must be
lowercase without whitespace. Object records have exactly `id` and `payload`;
duplicate IDs and duplicate JSON fields are rejected. Array order of objects
is irrelevant. The generator sorts them by binary typed ID.

`roots` are **codec verification entry points**, not a declaration that every
kind is a source root. A Blob can stand alone here. Maps are complete directory
roots, Conflicts must have root region `1`, and Revisions validate their source
and ordered parent closure. Path-region `0` conflicts must occur through a map
entry. Every supplied object must be reachable from these entry points; an
unrooted radix fragment has no complete basename context and is rejected.
Sharing objects is permitted, but each radix edge is validated with its own
consumed prefix. A Revision's source root can never be a Blob or a path conflict.

Two optional fields used by the pins are:

- `cases`: `[{"name": "empty-map", "root": "avc1-k02-…"}]`. Descriptive labels;
  multiple labels may name the same root (split/collapse returns identical IDs).
- `negative`: `[{"name": "trailing-leaf-byte", "objects": [...], "roots": [...]}]`.
  Each is a separate corpus that must be refused. Resource exhaustion is not
  counted as a successful malformed-input rejection.

Unknown top-level metadata is ignored. `--verify` validates canonicality, hashes,
complete owning closure, contextual map partitions, exact counts/maximal
prefixes, all names and nested path bounds, both signs of typed conflicts, and
revision roots/parents. Each structured payload also round-trips byte-for-byte.
It does not check signatures, infer missing ancestry, or establish provenance.

## Flat directory oracle

`--map` reads JSON from stdin and emits the minimal owning corpus of a newly
built map:

```json
{
  "objects": [{"id": "avc1-k01-<blob digest>", "payload": "6869"}],
  "bindings": [{"name": "66696c65", "entry": "<canonical Entry descriptor hex>"}]
}
```

```sh
uv run reference/check.py --map < directory-input.json > directory-corpus.json
uv run reference/check.py --verify directory-corpus.json
```

Names are raw basename bytes encoded as hex; entries are exact descriptor bytes,
including their tag, typed wire reference, and executable bit where applicable.
Provide owning dependencies in `objects`. Unused support objects are omitted
from the output. Duplicate names are refused, not overwritten. Input order is
irrelevant. Directory-valued entries can reference complete nested maps, so this
also supports differential tests on hierarchical sources.

The builder sorts the complete raw key set and partitions nibble suffixes from
scratch. The decoder independently recovers names and checks thresholds, branch
counts, nonempty groups, byte boundaries, and maximal prefixes; it does not
validate canonicality by rebuilding with the encoder. Self-checks compare
recovered bindings against flat byte-name oracles and include a hand-laid-out
33-entry wire fixture. This oracle does not implement persistent edits: its
split/collapse cases model identical final binding sets, not an incremental edit
algorithm.

## Experimental source export bundles

`bundle.py` independently parses the experimental repository's `AVCEXP01`
object-only export stream. It does not invoke Rust or change the canonical object
grammar:

```sh
uv run reference/bundle.py source.avcexp
uv run reference/bundle.py source.avcexp --json > source-corpus.json
uv run reference/check.py --verify source-corpus.json
```

The parser checks the explicit source root, strictly sorted unique typed-wire
IDs, bounded record lengths/count, completion marker, BLAKE3 checksum, and exact
EOF. The checksum covers every preceding byte **including** `AVCEND01`, but not
its own 32 bytes. After physical verification, `check.py` independently checks
all object IDs, canonical payloads, and the exact rooted owning closure. A valid
outer checksum cannot authorize a wrong object hash, missing dependency, or
surplus record. Only map/root-conflict source roots are accepted here, not Blob
or Revision entry points. `--json` emits the existing `{objects, roots}` corpus
only after all checks pass; failures return nonzero without a candidate corpus.

Defaults are 16 MiB per object payload, 256 MiB aggregate payload, and 100,000
objects. The whole-file default allows that payload plus framing:
`256 MiB + 41 × 100,000 + 89` bytes. Configure `--max-object-bytes`,
`--max-closure-bytes`, `--max-objects`, and `--max-file-bytes` independently.
Counts and lengths are checked before payload allocation; individual stream
reads are at most 64 KiB. An EOF check reads at most one additional byte and
refuses any trailing input. The delegated codec retains its traversal limits and
uses the bundle's aggregate byte cap for recovered-key workspace. Limits yield
refusal, never an authoritative partial result. Input must terminate.

This is an **experimental object-only source export**, excluding operation
receipts, authored history, named refs, selections, and workspace state. It is not a full
repository backup or a source-custody qualification. Tests construct streams
independently, including repaired-checksum attacks, without using the Rust
exporter; parent end-to-end tests can also feed real CLI exports to this parser.

## Selected authored-history streams

Use `--history` to accept `AVCHIS01` native history or `AVCHIS02` history with
portable Git origins. The default parser and `verify_stream` / `verify_file`
APIs still accept only `AVCEXP01`. Canonical v1 object payloads and native-only
`AVCHIS01` bytes are unchanged.

```sh
uv run reference/bundle.py history.avchis --history
uv run reference/bundle.py history.avchis --history --json > history-corpus.json
uv run reference/check.py --verify history-corpus.json
```

The `AVCHIS01` wire fields, in order, are:

- Eight-byte magic `AVCHIS01`.
- A 33-byte typed selected source root: map kind `02`/`03` or root Conflict `04`.
- A one-byte authored-head tag, exactly `0` or `1`. Tag `1` is followed by a
  33-byte typed Revision `05` head; tag `0` has no head field.
- A `u64le` record count, from 1 through 100,000. Each record is a 33-byte typed
  ID, a `u64le` canonical payload length, and that payload. IDs must be strictly
  sorted and unique.
- Eight-byte completion marker `AVCHEND1`, then the 32-byte BLAKE3 digest of
  every preceding byte, including the completion marker. EOF is mandatory.

`AVCHIS02` has the same source/head/object section, with magic `AVCHIS02` and a
mandatory present head. After the objects comes a nonzero `u64le` origin count
(at most 100,000), then strictly sorted unique records keyed by 33-byte native
Revision ID. Each record contains that ID, a 21-byte Git OID (`01` plus 20 raw
SHA-1 bytes), a `u64le` raw-commit length, and the exact raw commit. Its footer is
`AVCHEND2` followed by BLAKE3 over all preceding bytes, including that marker;
EOF is mandatory.

`verify_history_stream` / `verify_history_file` retain `{objects, roots}` for v1.
V2 additionally returns `origins`, with JSON records shaped as
`{"revision": "avc1-k05-…", "commit": "sha1:<40 lowercase hex digits>",
"raw_commit": "<lowercase hex>"}`. Verification roots are the selected source,
then the authored head when present. The selected source may differ from the
head Revision's source: uncommitted selection is not replaced by the authored
source. An absent head is valid for an unborn v1 selection, including an empty
map.

`encode_history(corpus, origins=None, limits=HISTORY_LIMITS)` returns verified
wire bytes, sorting input records. By default it uses `corpus.get("origins", [])`;
no origins emits v1, while any origins selects v2. Explicit `origins` overrides
the corpus field. The encoder refuses invalid closures or origins before
returning bytes. Existing source-only verification APIs are unchanged.

Records must be the **exact union** of both owning closures: all selected-source
dependencies, the head, every ancestor Revision, and all their source
dependencies. Shared objects occur only once. Negative conflict alternatives own
dependencies too. The checker rejects path conflicts in source-root contexts,
missing ancestry or sources, and surplus records even with a valid outer
checksum. Framing and bounded-read logic are shared with the source-only parser.

Origins must name zero-native-parent Revisions reachable from the selected head,
not merely present in the object array. The checker independently hashes exact
Git framing (`kind + ' ' + decimal length + NUL + raw`) with `hashlib.sha1`,
parses the established Git import header/person/message grammar, and recomputes
the complete native anchor ID. Its ChangeId is the first 16 BLAKE3 bytes of
`AVC-GIT-ANCHOR\0v1\0` followed by `01 14` and the raw commit OID. Author,
message, committer nanoseconds/timezone, and source must all match. Opaque extra
headers, including `avc-*` audit headers, grant no authority. Foreign parents
(at most 16 unique OIDs) remain opaque: they are not native ancestry and are
never fetched.

The original tree OID is reconstructed from resolved native Maps/Blobs using raw
names, exact modes, and Git's directory-name-plus-`/` ordering. Imported nested
empty directories are retained, even though ordinary Git export rejects them.
Reconstruction enforces 100,000 expanded entries per source, 256 components,
4,096 path bytes, 16 MiB per reconstructed object, and 100,000 objects/256 MiB
of unique reconstruction work across origins. Codec traversal/recovered-key
budgets also apply. No Git executable, repository, network, or Rust is required.
SHA-1 here checks framing/math; it is **not** a collision-detection security
oracle or proof of provenance.

History caps are 16 MiB per canonical payload or raw origin commit and 256 MiB
for their combined total; canonical objects and origins each cap at 100,000.
The same limit flags can tighten payload/object caps, but cannot relax them.
The history whole-file default accommodates v2 framing:
`256 MiB + 41 × 100,000 + 62 × 100,000 + 131` bytes.
`--max-file-bytes` overrides that framing budget. Both parsers emit JSON only
after full verification.

No named refs, local generations, request keys, instance identity, workspace
state, or operation receipts are encoded. This selected history transfer is not a full
repository backup. Tests lay out streams, Git trees/commits, and Revision
payloads independently of Rust and exercise the CLI's JSON output.

## Bounds and semantic decisions

`check.py` defaults: 64 MiB JSON input/aggregate payload/recovered-key workspace budgets,
16 MiB per payload, 100,000 objects, 1,000,000 traversal steps, and 1,024 nested
traversal levels. Set `--max-bytes`, `--max-object-bytes`, `--max-objects`,
`--max-visits`, and `--max-nesting` as needed (nesting maximum: 2,000). The byte
budget charges contextual key tuples conservatively so a small shared radix DAG
cannot expand without bound. Fixed grammar field caps remain mandatory. These
limits intentionally refuse some otherwise-valid large inputs; refusal gives no
authoritative result. JSON is bounded before parsing; field lengths are checked
before slicing. Blob contents, including symlink targets, remain uninterpreted.

Directory summaries are derived from validated closure, including *negative*
conflict alternatives and empty directory bindings. They are recomputed for each
verification, never imported from JSON. All name/path limits apply to the whole
source, not just each object in isolation. Multiplicity does not change path
bounds or permit dropping an owning term.

Signed normalization is an algebra helper: it flattens input conflicts, inverts
signs for negative operands, combines multiplicities, and cancels only
byte-identical descriptors. `2A - B` remains unresolved even when its two positive
occurrences agree. Tests exercise repeated rebase and revert cancellation
without conflating this with section E's intentionally lossy clean equality/base
fast paths.

`Store.merge_opaque` is a tiny semantic oracle for already-validated entry values
when no specialized directory/content driver applies. It first checks `L=R`,
`L=B`, and `R=B`. If those do not resolve the result and the base is a Conflict,
it returns `Pending("UnresolvedBase")`, without producing an object. It must not
subtract the unresolved base: resolving `C = X + Y - Absent` independently to X
and Y would otherwise fabricate Absent. A resolved base still permits exact
cancellation and flattening of positive-side conflicts. Direct calls to `signed`
do not enforce this merge policy and must not be mistaken for an authoritative
merge operation. The reference does not implement text/directory merge drivers,
ancestry-base selection, or edits. Equal hashes do not imply moves; only explicit
validated witnesses can authorize a move.

For semantic cross-checking, consulted Jujutsu's primary
[first-class conflict technical document](https://github.com/jj-vcs/jj/blob/c746b82313b9c23199d376eb45899019258c502e/docs/technical/conflicts.md)
on 2026-11-05. It explains substitution/cancellation and explicitly identifies
same-change resolution as lossy, citing
[issue #6369](https://github.com/jj-vcs/jj/issues/6369). This supports the separation
above; Jujutsu's implementation, ordering, and merge policies are not imported.

### Source roots are never absent

Every term of a region `1` conflict, positive or negative, must be a Directory.
Absent is forbidden on both signs; deleting all source bindings produces the
empty map, never a missing root. Region `0` retains Absent at ordinary paths.
Region `1` conflicts are legal only as source roots, and region `0` conflicts
only as Entry descriptors. The root-Absent wording tension identified in the
initial reference has been resolved by this directory-only rule. The named
`root-conflict-positive-absence` and `root-conflict-negative-absence` rejection
vectors pin both sides of the correction.

Total multiplicity at most 64 together with signed balance exactly `+1` means
the greatest realizable total is **63**, necessarily odd. The reference checks
the written inequalities, including the 63-valid/65-invalid boundary.
