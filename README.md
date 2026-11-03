# AVC

**Experimental native source control for developers and coding agents.** AVC is a clean-sheet, local-first VCS intended to combine trustworthy offline source ownership, editable history, fast selective clones, native agent/source APIs, and easy adoption of existing Git repositories.

**Status (November 3, 2026): architecture redesign in progress; no usable native repository operations or safe source custody yet.** The previous elaborate v0 contract has been replaced by a shorter working design after re-evaluating Git/Pando, Scion/Mononoke, Jujutsu, GitButler and other systems. Its object format, merge algebra and storage layout are **not frozen**. This is not a released Git alternative.

## Design direction

- **Native, not Git-backed.** Immutable content-addressed source trees, authored revision history and stable logical change identities. Git import/export and upstream tracking are adoption boundaries, not canonical source authority.
- **Clone the current useful source first.** Select a remote branch or revision, fetch a bounded amount of history and the needed source, then expand missing history/files explicitly or on demand. Small projects should remain easy to use with normal editors; large monorepos and agents should avoid full downloads and checkouts when possible.
- **Exact offline and recovery claims.** A complete native repository and backup are independently reconstructible; a thin clone never mistakes missing promised objects for absent source. Separate private checkpoints from authored commits and remote publication, with guarded restore and operation-log undo.
- **Independent and composed work.** Multiple agent workspaces or checkout-free conditional edits; optional GitButler-like independent and stacked changes in one *derived* view. A shared mutable folder is not required for concurrent agents.
- **Modern physical optimizations.** BLAKE3 as a logical hashing candidate; compare Zstandard immutable segments and SQLite-only storage, directory layouts, merge representations and transfer batching under real workloads. Codec choice is not itself a clone-speed proof.
- **Optional native hosting.** Content-addressed object APIs, reusable cacheable transfers, derived history indexes, per-ref conditional updates and full local operation without a server.

## Current source and safety

The Rust library contains parts of the **older experimental codec**, streaming object hashing, in-memory Merkle map construction and owning-closure verification. Those routines and the Python/Node reference fixtures do **not** implement a durable source-control engine. The executable currently exposes informational flags; operational commands refuse. Previous Git-wrapper operations have been retired.

**Do not use AVC to protect important work yet.** Keep existing Git repositories, backups and old `.avc` recovery data intact. There is no automatic migration, clone/import, reliable repository backup or crash-safe restore implemented.

## Maintained documents

- [Architecture](docs/design.md) — current working source/identity, clone, storage, concurrency, Git interoperability and safety design; explicitly unsettled selections.
- [Workflows](docs/ux.md) — proposed developer/agent interactions, clone policy and safety disclosure; command spellings illustrative.
- [Implementation and qualification](docs/roadmap.md) — experiments, native vertical slices, Git on-ramp, custody gates and next local session.
- [Comparative research](docs/research.md#31-fresh-native-vcs-review-git-adoption-selective-clones-and-parallel-changes--november-3-2026) — primary-source evidence and dated previous investigations. Older sections are historical; they no longer lock decisions.
- [Reference experiments](docs/experiments/) — existing fixtures and bounded studies for the prior draft, not a released format or complete performance validation.

## Building/checking the current experimental crate

```sh
cargo test --workspace --locked --offline
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
```

These verify the existing limited crate and tests, not the new architecture or full repository operation. Follow the [roadmap](docs/roadmap.md) for the next changes and release gates.

## License

MIT. See [LICENSE](LICENSE).
