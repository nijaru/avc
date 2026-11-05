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
pinned in the scripts' PEP 723 headers.

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

## Bounds and semantic decisions

Defaults: 64 MiB JSON input/aggregate payload/recovered-key workspace budgets,
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
