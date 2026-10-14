# AVC

Experimental design for a native, general-purpose version-control system serving people and concurrent coding agents.

The goal is better complete workflows: inspect and edit exact source, recover experiments, isolate independent workers, and integrate reviewed changes without making each private edit a public event. Git is an interoperability target, not a required underlying engine.

## Status

**The native system is not implemented.** The current Rust 0.0.1 code is an earlier Git-command prototype with a separate JSON operation log. It does not establish the proposed durability, recovery or concurrency guarantees. Keep independent backups.

The current investigation compares three complete models: persistent source revisions, causal change graphs with materialized views, and parent-relative changeset logs with derived trees. A snapshot-oriented native model is the concrete reference candidate, not a frozen canonical format or a demonstrated universal optimum. The earlier State/WorkVersion/Application and change-graph recommendations are not accepted requirements.

## Design and evidence

- [Research](docs/research.md): user/workload requirements, recent primary sources, alternatives and evidence limits.
- [Reference design](docs/design.md): data model, transactions, merge behavior, working copies, APIs and publication contracts.
- [Roadmap](docs/roadmap.md): focused comparisons, implementation sequence and release gates.
- [Contract probes](docs/experiments/vcs_contract_probes.py): reproducible counterexamples behind several design corrections; [recorded results](docs/experiments/vcs_contract_results.json).

The probes are not a native performance benchmark or power-loss qualification. The physical store, canonical tree format and ultimate history model remain subject to the documented comparisons.

Tavi remains an unapproved name candidate. Repository, executable, crate and metadata paths retain AVC.

## Development

```sh
cargo test
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```

Run the research probes separately with Python 3.11+ and Git:

```sh
python docs/experiments/vcs_contract_probes.py
```

## License

MIT. See [LICENSE](LICENSE).
