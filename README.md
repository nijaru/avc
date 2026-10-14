# AVC

**Experimental native version control for people and concurrent coding agents.**

AVC is a clean-sheet, local-first VCS design. Its core idea is to separate **exact source state**, **independently authored changes**, and **the application of those changes to a particular target state**. That allows workers to keep their original authored work while a composition operation brings selected results together for verification and publication.

The goal is to improve entire developer workflows, not merely Git commit speed or command syntax: safe recovery, easy independent experiments, cheap workspaces and checkout-free queries, deterministic conflict handling, and deliberate integration with less unnecessary coordination.

**Git and Jujutsu are benchmarks and design references, not the authoritative AVC engine.** Git import/export is planned for adoption, while ordinary local editing and recovery must work without a server. A native implementation must demonstrate meaningful improvements and pass strict source-integrity tests before it should be trusted with irreplaceable work.

## Status

**Design only. The native system is not implemented.** The existing Rust 0.0.1 prototype still uses Git commands and an independently appended JSON log. It does not provide the durability, automatic recovery, concurrent workspace safety, or composition semantics described in the new architecture. Keep independent backups and do not use it as the sole copy of important work.

The current design commits to the logical distinctions and safety requirements while deliberately leaving patch algebra versus canonical snapshots, physical object-store layout, wide-directory representation, and other measured algorithm choices open to focused experiments.

## Documents

- [Architecture](docs/design.md) — source State, authored WorkVersion, context-specific Application, workspace and publication semantics, storage contracts and safety invariants.
- [Research](docs/research.md) — independent comparison with Git, Jujutsu, Pijul/Darcs, Scion, Sapling and other systems, including reasons to reconsider the model.
- [Roadmap](docs/roadmap.md) — competing prototypes, acceptance criteria, Git exchange, durability verification and end-to-end benchmarks.

**Naming is not settled.** Tavi is a provisional candidate only; the repository, crate, CLI and metadata directory remain AVC.

## Existing prototype checks

~~~sh
cargo test
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
~~~

These checks apply to the old prototype, not to the future native correctness contracts.

## License

MIT. See [LICENSE](LICENSE).
