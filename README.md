# AVC

**Experimental native version control for humans and agents.**

AVC is being redesigned from first principles as a **local-first VCS with its own source representation, storage, history, transaction and synchronization protocols**. Git and Jujutsu are important references, not its canonical engines.

The goal is not a marginally nicer Git client. The value proposition is reliable automatic recovery, stable logical changes, inexpensive isolated workspaces, checkout-free source operations, safe multiworker composition and deliberate validated integration—without requiring a hosted service for ordinary use.

## Status

**This architecture is a design, not working code.** The Rust 0.0.1 prototype still invokes Git and separately appends a JSON operation log. It has not established the proposed native durability, recovery, garbage collection, interoperability, or concurrency guarantees. Keep independent backups; do not use this prototype as the only copy of important work.

The new architecture will replace the old Git-command loop rather than preserve it as a legacy core. Git import/export and existing-host collaboration remain essential adoption paths, but they must not constrain native objects and operations.

## Documents

- [Architecture](docs/design.md): native source model, transaction and recovery rules, workspaces, agent interfaces and collaboration semantics.
- [Research](docs/research.md): tradeoffs against Git/Jujutsu, lessons from Scion, Sapling and Pijul, limitations and naming.
- [Roadmap](docs/roadmap.md): implementation milestones and required data-safety and performance evidence.

**Tavi** remains a proposed name, not an approved rename. The repository, crate, executable and .avc path retain the AVC name.

## Current development checks

    cargo test
    cargo fmt --check
    cargo clippy --all-targets --all-features -- -D warnings

These checks are not substitutes for native crash-injection, state-model and interoperability verification.

## License

MIT.
