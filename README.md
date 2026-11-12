# AVC

**Experimental native, local-first version control for developers and coding agents.**

AVC is building immutable persistent source trees, editable history, offline recovery, selective native clones, checkout-free conditional edits and permanent Git interoperability—without a Git-backed native engine.

## Status — November 12, 2026

The new Rust library implements the current v1 canonical objects, BLAKE3 IDs, persistent radix edits, queries, Merkle diffs, witnessed moves/copies, bounded text merging and signed source conflicts. An independent Python codec checks fresh vectors and malformed objects; the old prototype and its format have not been reinstated.

The **experimental repository and CLI** support explicit filesystem leaf and recursive directory capture, conditional source selection, authored commits, tip amend, scoped metadata undo, single-change rebase, local SHA-1 Git import/candidate export, first-parent history, immutable retry receipts, verification, source-only and selected-history export/import, and no-clobber materialization into a fresh directory. Mutations require `--experimental`. Source edits and authored history are separate: `source` selects exact bytes; `commit` authors that selection without scanning the filesystem. One bounded SQLite database commits payloads, selection and receipts together. Source loads/admissions verify complete source closures; ordinary history reads check metadata without rereading every historical file. Pure-engine incremental costs are not CLI performance claims; source-edit commands still load complete source closures.

**Keep independent backups.** AVC is experimental (`0.0.1`), with an unreleased format. Named branches, workspace recovery and Git/native synchronization are not implemented. Durable writes rely on SQLite's documented WAL/FULL contract and scoped integration tests, not hardware certification or power-loss immunity. Default `export`/`import` transfers one exact source, **not authored history**. Explicit `--history` transfers the selected source and authored head with all owned native ancestry, but not local receipts, instance identity, workspaces or Git metadata; Git-anchored history therefore refuses this export rather than drop its origin. Do not copy the main database alone from an active store: its WAL may own committed data. Existing Git repositories and legacy `.avc` data are never automatically converted or removed.

The discarded v0 implementation and prior studies remain available at [the pinned pre-cleanup commit](https://github.com/nijaru/avc/tree/af66a63898f383b596e4828199dc625a80e28a72), as historical evidence rather than current contracts.

A [small local Git round trip](docs/git.md#available-local-round-trip) is available: import an exact Git tip, author natively, then export candidate objects with the exact original Git parent. It does not publish refs or use the network. Explicit file/symlink and recursive directory capture supports edit/review/commit without rereading live files. Opt-in removal of missing bindings requires complete parent-directory coverage; directly selected missing paths leave sibling payloads untouched. [Next](docs/roadmap.md#delivery-order-from-the-current-implementation) is broader workflow support.

## Build and try

Use a current stable Rust toolchain. Run from this checkout with fresh demo paths:

```sh
cargo build --locked
mkdir -p tmp/demo tmp/restored
./target/debug/avc --repository tmp/demo --experimental init
./target/debug/avc --repository tmp/demo --experimental capture --from . --recursive \
  --expected-generation 0 --request-key demo-source README.md src
./target/debug/avc --repository tmp/demo show README.md
./target/debug/avc --repository tmp/demo --experimental commit \
  --author 'Example <example@example.test>' -m 'Add README' \
  --expected-generation 1 --request-key demo-commit
./target/debug/avc --repository tmp/demo log
./target/debug/avc --repository tmp/demo status
./target/debug/avc --repository tmp/demo verify
./target/debug/avc --repository tmp/demo --experimental amend \
  --author 'Example <example@example.test>' -m 'Refine description' \
  --expected-generation 2 --request-key demo-amend
./target/debug/avc --repository tmp/demo --experimental history undo demo-amend \
  --expected-generation 3 --request-key demo-undo
./target/debug/avc --repository tmp/demo export --history --output tmp/demo.avc
./target/debug/avc --repository tmp/restored --experimental init
./target/debug/avc --repository tmp/restored --experimental import --history \
  --input tmp/demo.avc --request-key demo-restore
./target/debug/avc --repository tmp/restored log
./target/debug/avc --repository tmp/restored --experimental restore --to tmp/fresh-source
```

`init` refuses any existing `.avc`; export refuses an existing output file. The current physical schema is 10; older stores are refused unchanged, without automatic migration. Canonical v1 objects and source export bytes are unchanged. `capture` pins and selects explicit existing file/symlink leaves; `--recursive` also includes complete directory descendants (including new files and empty directories). Paths are relative to `--from`; `.` selects that whole root only with `--recursive`. Unselected source and the authored head are preserved. Review its before/after roots with `diff`, then `commit`; later filesystem edits are not silently included. `history undo CAPTURE_KEY` cancels that selection while it still owns the current generation, without authoring or changing live files. Ordinary capture refuses missing selected paths. Explicit `--recursive --remove-missing` removes old raw-name bindings proven absent from a completely enumerated existing directory, reporting each removal before commit. Tracked missing files, symlinks or directories can be selected directly when their parent exists; only sibling names are enumerated, not sibling payloads. Untracked typos, missing parents, directory-to-leaf replacements and failed reads still refuse. Hardlinked regular files, ignore rules and implicit tracked-file selection are not supported. Administration entries are excluded from recursive scans and any corresponding selected source bindings are retained; explicit administration paths and symlink parents refuse. The receipt reports inclusion/removal policy, observed counts, removed bindings and unobserved administration scopes. Observation witnesses detect observed races; they do not prove a simultaneous filesystem snapshot. `source put` remains available for explicitly supplied bytes. Source edits and commits share one selection generation: source edits preserve the authored head, while commits preserve selected source and advance the head. Inspect `status` for the current generation and `receipt KEY` to reconcile an unknown response. An identical retry returns its original outcome—even after later changes; changed operation kind or request identity refuses.

Use `--history` on both export and import to preserve native authored history. It also preserves selected-but-uncommitted source separately from the head's source. Import requires an untouched initialized store and creates a fresh local instance/generation and retry receipt; it does not import the original store's operation log. Both modes reject incomplete/corrupt streams and have explicit aggregate limits. `export --history` cannot be combined with `--root`; omit `--history` for the existing source-only format.

`commit` requires `--author` (or `AVC_AUTHOR`) and an explicit message; unchanged/empty-source commits are allowed. New commits receive random ChangeIds and UTC timestamps; retries retain both. `log` defaults to 20 first-parent entries; use `--revision ID` and `--limit N` to select a start and bound. `verify --root REVISION_ID` checks the complete bounded ancestry/source graph. See [storage](docs/storage.md) for limits and retention.

`amend` uses the same explicit author/message/generation/request flags as `commit`. It authors selected source into a new Revision with the existing ChangeId and ordered parents, retaining the predecessor locally; it refuses an unborn head. Unchanged and empty source outcomes are allowed and reported. `history undo OPERATION_KEY --expected-generation N --request-key KEY` restores that operation's recorded before-source/head only while it still owns the current selection/generation. Undo advances generation and has its own retry receipt; it does not alter live files or remotes. Selected-history transfer excludes private undo receipts and superseded revisions outside the selected owning graph.

`rebase --onto REVISION_ID --expected-generation N --request-key KEY` replays one clean selected change onto an exact stored revision. It preserves ChangeId, author/message and authored timestamp, with the target as its sole parent. The original change must have zero or one parent; dirty selected source, a target ancestry containing the same ChangeId, and unsafe differing resolutions of a conflicted comparison base refuse. Unchanged/empty outcomes are explicit; unresolved source conflicts may be published without touching files. This is not descendant restacking or named-ref rebase. Both input history graphs and the result must fit the current aggregate bounds.

`git import --git-dir DIR --commit OID --request-key KEY` requires an untouched store and a complete current SHA-1 Git source. `git origins` inspects retained foreign boundaries. `git export --git-dir DIR` requires clean selected source/head and emits candidate objects only; it refuses conflicts, empty directory entries, missing origins or nonrepresentable identities. Git timestamps use declared second precision with exact native nanoseconds retained in audit headers. Both import and export require `--experimental`. See the [Git subset, example and portability limit](docs/git.md#available-local-round-trip); this is not Git/native synchronization.

Available commands are listed by `avc --help` and `avc source --help`. Source operations include empty directories, exact file/symlink bytes, remove, move/copy and explicit-base merge. They leave ordinary project files untouched. A source candidate is not an authored revision or a claim that tests passed. `restore` requires an absent destination and non-symlink parents, preserves raw file/symlink bytes, mode and empty directories, and registers no workspace. Errors can leave an incomplete new directory; it is never cleaned up or overwritten automatically. Fresh materialization currently requires Unix dirfd operations.

## Checks

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
uv run reference/check.py --vectors
AVC_REFERENCE_CHECK=1 cargo test --workspace --locked --offline
cargo test --release --workspace --locked --offline
```

`uv` acquires the reference's pinned BLAKE3 dependency on first use. `AVC_REFERENCE_CHECK=1` also checks Rust-generated source, CLI-authored revision ancestry and actual history exports across languages. Git boundary tests use isolated local Git fixtures; no external repository/network is needed. See [the roadmap](docs/roadmap.md) for current limits and next implementation slices, and [the reference README](reference/README.md) for independent corpus interfaces.

## Design owners

- [Architecture](docs/design.md): source authority, invariants and component boundaries.
- [Canonical format and merge](docs/format.md): exact typed objects, persistent radix layout, conflicts and conditional operations.
- [Storage and recovery](docs/storage.md): durable admission, physical formats, GC, observation, materialization and backup.
- [Native synchronization](docs/sync.md): partial availability, proofs, transfer and ref publication.
- [Git compatibility](docs/git.md): native import/export versus explicitly foreign partial views; optional future Git protocol frontend.
- [User workflows](docs/ux.md): intended human/agent interactions; distinguish them from today's limited CLI.
- [Roadmap](docs/roadmap.md): implementation progress and release gates.
- [Research](docs/research.md): existing evidence, rejected alternatives and demonstrated corrections.

MIT. See [LICENSE](LICENSE).
