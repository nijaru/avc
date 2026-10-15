# AVC

**Experimental native version control for people and coding agents.**

AVC targets ordinary offline source development as well as remote workspaces, CI, containers, microVMs, VPSs and large agent fleets. The intended advantage is a safe end-to-end workflow: recover unfinished edits, make independent work cheap, inspect exact source without cloning/checkout when unnecessary, preserve conflicts, and integrate deliberately.

**AVC is not Git-based. Git compatibility, Git object formats, and Git protocol are not architectural requirements.** Existing systems inform the research but do not determine the design.

## Design direction

The leading reference is **immutable source states backed by persistent Merkle trees**, with immutable revision ancestry, stable logical change identities, independently versioned workspaces, typed conflicts and separate bounded private recovery.

A revision directly identifies the exact source that tools inspect, build, test and share. Temporary edits can be captured cheaply without becoming permanent authored history or public events. A native library/source API can work locally offline or through an optional hosted service, using sparse object reads or bulk transfer as appropriate.

**This is a concrete proposed architecture, not a proven optimum.** Parent-relative canonical changesets and causal patch graphs remain competitors where their extra semantics or cheap admission justify the costs. A canonical source-tree encoding and physical store must pass the [documented experiments](docs/roadmap.md) before format stability.

## Status and documents

**No native AVC engine has been implemented.** Rust 0.0.1 is still an old Git-command wrapper with a separate JSON log. It does not establish the proposed durability, recovery or concurrency guarantees; keep independent backups.

- [Architecture](docs/design.md) — source state and revision contracts; merge, recovery, workspace, API, storage and sync semantics.
- [Research](docs/research.md) — primary sources, alternative histories and reasons for the current decision.
- [Roadmap](docs/roadmap.md) — executable model, canonical-history comparisons and native implementation gates.
- [Contract probes](docs/experiments/vcs_contract_probes.py) and [results](docs/experiments/vcs_contract_results.json) — narrow reproducible counterexamples, not native performance measurements.
- [Source-model oracle](docs/experiments/source_models_oracle.py) — 14 deterministic in-memory tests comparing the semantics of snapshot-root and validated delta histories; not a performance benchmark.
- [Executable source-model oracle](docs/experiments/source_models_oracle.py) — independent source-value tests and a minimal snapshot-versus-delta reconstruction check; not a performance or durability benchmark.

The project remains named AVC. Tavi/Delos are unapproved naming suggestions, not implementation decisions.

## Existing prototype checks

    cargo test
    cargo fmt --check
    cargo clippy --all-targets --all-features -- -D warnings

## License

MIT. See [LICENSE](LICENSE).
