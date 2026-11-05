# AVC

**Experimental native, local-first version control for developers and coding agents.**

AVC is building immutable persistent source trees, editable history, offline recovery, selective native clones, checkout-free conditional edits and permanent Git interoperability—without a Git-backed native engine.

## Status — November 5, 2026

The new Rust library implements the current v1 canonical objects, BLAKE3 IDs, persistent radix edits, queries, Merkle diffs, witnessed moves/copies, bounded text merging and signed source conflicts. An independent Python codec checks fresh vectors and malformed objects; the old prototype and its format have not been reinstated.

An **experimental source-only repository and CLI** now support SQLite-backed conditional source selection, immutable retry receipts, reopening, verification, self-contained object export/import and no-clobber materialization into a fresh directory. Repository mutations and fresh materialization require `--experimental`. These commands do **not** scan or modify an existing working tree, author revisions, register workspaces, checkpoint editor work, import Git history, or provide a qualified backup service. The physical store is deliberately bounded and currently revalidates complete closures on load/admission; pure-engine incremental costs are not CLI performance claims.

**Do not entrust irreplaceable source to AVC.** The format remains unreleased and unfrozen. Full fault injection, power-loss/platform qualification, GC, compression/large-file streaming, working-directory capture/recovery, authored history and Git/native synchronization remain incomplete. Keep independent backups. Existing Git repositories and legacy `.avc` data are never automatically converted or removed.

The discarded v0 implementation and prior studies remain available at [the pinned pre-cleanup commit](https://github.com/nijaru/avc/tree/af66a63898f383b596e4828199dc625a80e28a72), as historical evidence rather than current contracts.

## Build and try the source slice

Use a current stable Rust toolchain. Run from this checkout with fresh demo paths:

```sh
cargo build --locked
mkdir -p tmp/demo tmp/restored
./target/debug/avc --repository tmp/demo --experimental init
./target/debug/avc --repository tmp/demo --experimental source put README.md \
  --file README.md --expected-generation 0 --request-key demo-readme
./target/debug/avc --repository tmp/demo show README.md
./target/debug/avc --repository tmp/demo verify
./target/debug/avc --repository tmp/demo export --output tmp/demo.avc
./target/debug/avc --repository tmp/restored --experimental init
./target/debug/avc --repository tmp/restored --experimental import \
  --input tmp/demo.avc --request-key demo-restore
./target/debug/avc --repository tmp/restored --experimental restore --to tmp/fresh-source
```

`init` refuses any existing `.avc`; export refuses an existing output file. `source put` reads an explicitly supplied file, not a simultaneous filesystem snapshot. Source operations require an exact selection generation and a request key. Inspect `status` for the current generation and `receipt KEY` to reconcile an unknown response. Retrying the same request returns its original outcome; changed request identity refuses.

Available commands are listed by `avc --help` and `avc source --help`. Source operations include empty directories, exact file/symlink bytes, remove, move/copy and explicit-base merge. They leave ordinary project files untouched. A source candidate is not an authored revision or a claim that tests passed. `restore` requires an absent destination and non-symlink parents, preserves raw file/symlink bytes, mode and empty directories, and registers no workspace. Errors can leave an incomplete new directory; it is never cleaned up or overwritten automatically. Fresh materialization currently requires Unix dirfd operations.

## Checks

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo test --workspace --locked --offline
uv run reference/check.py --vectors
AVC_REFERENCE_CHECK=1 cargo test --locked --offline
```

`uv` acquires the reference's pinned BLAKE3 dependency on first use. The last command also runs cross-language checks of Rust-generated source. Git boundary tests use isolated local Git fixtures; no external repository/network is needed. See [the roadmap](docs/roadmap.md) for qualification limits and remaining gates, and [the reference README](reference/README.md) for independent corpus interfaces.

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
