# AVC

**Experimental native version control for developers and coding agents.** AVC aims to combine correct local/offline source ownership, efficient checkout-free source APIs, editable history, native partial repositories, and practical Git adoption **without being Git-backed**.

**Status (November 3, 2026): the core design has been selected for implementation, but AVC is not yet a usable or reliable VCS.** No canonical format is frozen. Existing Rust code implements only part of a superseded experimental object codec and in-memory Merkle/verifier; the CLI still rejects all operational commands. Keep independent Git backups of important source and preserve old `.avc` recovery data.

## Architecture

- **One exact source authority:** immutable BLAKE3-identified, persistent Merkle source trees; authored revisions reference exact source roots. Changed-path descriptions and provenance are derived or checked hints, not another competing history.
- **Cheap incremental edits:** path-copy affected radix-map nodes, not full-tree construction. Stable logical change identities and a separate durable operation log support amend, rebase, scoped undo and recoverable local work.
- **Fast source-first clones:** get the current selected tip/source without all old history; native partial clients can fetch additional files and history on demand or ask a full holder to validate conditional edits. Full offline custody is explicit and verifiable.
- **Honest Git bridge:** normal Git imports hydrate and convert a complete current source snapshot. An enormous filtered Git repository may remain a *foreign partial view* until its source can actually be converted; Git object IDs cannot be translated into native BLAKE3 blob IDs without content.
- **Independent and composed changes:** optional GitButler-like stacks/derived views, Jujutsu/Sapling-like history editing, and safe independent agent source operations. No mandatory daemon, hosted service, shared writer, FUSE or agent harness.
- **Modern physical storage:** one SQLite transactional metadata/root authority plus staged immutable payload segments; Zstandard, batch fetch, caching and compaction are physical optimization candidates, not canonical identities. Durability must be tested against a SQLite-only control.

## Project documents

- [Design](docs/design.md): current authoritative logical/physical architecture and correctness boundaries.
- [Workflows](docs/ux.md): proposed human/agent tasks and truthful clone/recovery behavior.
- [Roadmap](docs/roadmap.md): implementation sequence, narrow pre-format-freeze comparisons and custody gates.
- [Research](docs/research.md): independent steelman, concrete evidence and overturn conditions. Previous chronological research and its original reproduction instructions are [pinned in Git history](https://github.com/nijaru/avc/blob/94dd9921ee8a93b89551cbecf53bb14f16f08b01/docs/research.md).
- [Historical experiments](docs/experiments/): bounded reference studies of the *older draft*, not native performance or proof of safe custody.

## Current crate

The present Rust sources are experimental and must be migrated to the new model as a cohesive change. Their `FlatMap` builder reconstructs an entire directory and the verifier walks/reconstructs complete maps; these are not acceptable as the incremental production engine. Do not treat old golden vectors as a stable compatibility contract.

The current limited checks are:

```sh
cargo test --workspace --locked --offline
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
```

These commands were **not run during the November 3 design-only remote review**; they exercise the older prototype, not a functioning VCS. Follow the [roadmap](docs/roadmap.md) for the first safe local vertical slice.

## License

MIT. See [LICENSE](LICENSE).
