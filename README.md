# AVC

**Experimental native, local-first version control for developers and coding agents.**

AVC is building immutable persistent source trees, editable history, offline recovery, selective native clones, checkout-free conditional edits and permanent Git interoperability—without a Git-backed native engine.

## Status — November 9, 2026

The new Rust library implements the current v1 canonical objects, BLAKE3 IDs, persistent radix edits, queries, Merkle diffs, witnessed moves/copies, bounded text merging and signed source conflicts. An independent Python codec checks fresh vectors and malformed objects; the old prototype and its format have not been reinstated.

The **experimental repository and CLI** support conditional source selection, authored commits, first-parent history, immutable retry receipts, verification, source export/import and no-clobber materialization into a fresh directory. Mutations require `--experimental`. Source edits and authored history are separate: `source` selects exact bytes; `commit` authors that selection without scanning the filesystem. One bounded SQLite database commits payloads, selection and receipts together. Source loads/admissions verify complete source closures; ordinary history reads check metadata without rereading every historical file. Pure-engine incremental costs are not CLI performance claims.

**Keep independent backups.** AVC is experimental (`0.0.1`), with an unreleased format. Amend/rebase, named branches, working-directory capture/recovery and Git/native synchronization are not implemented. Durable writes rely on SQLite's documented WAL/FULL contract and scoped integration tests, not hardware certification or power-loss immunity. `export`/`import` transfers one exact source, **not authored history or repository metadata**. Do not copy the main database alone from an active store: its WAL may own committed data. Existing Git repositories and legacy `.avc` data are never automatically converted or removed.

The discarded v0 implementation and prior studies remain available at [the pinned pre-cleanup commit](https://github.com/nijaru/avc/tree/af66a63898f383b596e4828199dc625a80e28a72), as historical evidence rather than current contracts.

## Build and try

Use a current stable Rust toolchain. Run from this checkout with fresh demo paths:

```sh
cargo build --locked
mkdir -p tmp/demo tmp/restored
./target/debug/avc --repository tmp/demo --experimental init
./target/debug/avc --repository tmp/demo --experimental source put README.md \
  --file README.md --expected-generation 0 --request-key demo-readme
./target/debug/avc --repository tmp/demo show README.md
./target/debug/avc --repository tmp/demo --experimental commit \
  --author 'Example <example@example.test>' -m 'Add README' \
  --expected-generation 1 --request-key demo-commit
./target/debug/avc --repository tmp/demo log
./target/debug/avc --repository tmp/demo status
./target/debug/avc --repository tmp/demo verify
./target/debug/avc --repository tmp/demo export --output tmp/demo.avc
./target/debug/avc --repository tmp/restored --experimental init
./target/debug/avc --repository tmp/restored --experimental import \
  --input tmp/demo.avc --request-key demo-restore
./target/debug/avc --repository tmp/restored --experimental restore --to tmp/fresh-source
```

`init` refuses any existing `.avc`; export refuses an existing output file. The current physical schema is 3; older stores are refused unchanged, without automatic migration. Canonical v1 objects and source export bytes are unchanged. `source put` reads an explicitly supplied file, not a simultaneous filesystem snapshot. Source edits and commits share one selection generation: source edits preserve the authored head, while commits preserve selected source and advance the head. Inspect `status` for the current generation and `receipt KEY` to reconcile an unknown response. An identical retry returns its original outcome—even after later changes; changed operation kind or request identity refuses.

`commit` requires `--author` (or `AVC_AUTHOR`) and an explicit message; unchanged/empty-source commits are allowed. New commits receive random ChangeIds and UTC timestamps; retries retain both. `log` defaults to 20 first-parent entries; use `--revision ID` and `--limit N` to select a start and bound. `verify --root REVISION_ID` checks the complete bounded ancestry/source graph. See [storage](docs/storage.md) for limits and retention.

Available commands are listed by `avc --help` and `avc source --help`. Source operations include empty directories, exact file/symlink bytes, remove, move/copy and explicit-base merge. They leave ordinary project files untouched. A source candidate is not an authored revision or a claim that tests passed. `restore` requires an absent destination and non-symlink parents, preserves raw file/symlink bytes, mode and empty directories, and registers no workspace. Errors can leave an incomplete new directory; it is never cleaned up or overwritten automatically. Fresh materialization currently requires Unix dirfd operations.

## Checks

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
uv run reference/check.py --vectors
AVC_REFERENCE_CHECK=1 cargo test --workspace --locked --offline
cargo test --release --workspace --locked --offline
```

`uv` acquires the reference's pinned BLAKE3 dependency on first use. `AVC_REFERENCE_CHECK=1` also checks Rust-generated source and CLI-authored revision ancestry across languages. Git boundary tests use isolated local Git fixtures; no external repository/network is needed. See [the roadmap](docs/roadmap.md) for current limits and next implementation slices, and [the reference README](reference/README.md) for independent corpus interfaces.

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
