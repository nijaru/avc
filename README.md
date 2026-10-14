# AVC

AVC is an experimental **native, local-first version control system** intended for ordinary software development and increasingly concurrent human/agent workflows. Git interoperability is an adapter, not its storage or history foundation.

## Architectural direction

The leading reference design is **minimal immutable changesets with content-addressed file data**, plus independently maintained source-tree and history indexes. Ordinary editing and history work offline; large hosted deployments may run the same source operations remotely and fetch only needed objects.

The distinctions matter:

- A **changeset** records paths changed relative to its first parent, rather than making construction of a complete tree index part of every commit.
- A **source view** is a persistent Merkle-indexed representation of the files at that changeset; it is derived from canonical history and can be rebuilt.
- **Workspaces** are independent mutable directories over shared immutable content. A **target** is an intentionally updated shared pointer; unrelated local work must not contend on one global head.
- **Private recovery captures**, authored history, review proposals and accepted target updates have different lifetimes and side effects.
- Source queries and most operations can execute **without creating a checkout**.

**This is a coherent implementable reference architecture, not a demonstrated optimum.** A canonical snapshot-first design remains the principal challenger, especially when most revisions are immediately materialized or read. Causal patch graphs are a further alternative for stronger undo/commutation semantics. The source format will not be frozen until focused comparisons settle those questions.

## Current implementation

**None of the native architecture above is implemented.** The Rust 0.0.1 code is an earlier Git-command prototype with a separate JSON operation log. It does not provide the native durability, recovery, concurrency, or publication guarantees. Keep independent backups; do not use it as the sole copy of important work.

- [Design](docs/design.md): exact proposed object, query, merge, workspace and transaction contracts.
- [Research](docs/research.md): primary-source evidence, competing models, trade-offs and reversal criteria.
- [Roadmap](docs/roadmap.md): first working slice and tests that must succeed before commitment or release.
- [Contract probes](docs/experiments/vcs_contract_probes.py) and [results](docs/experiments/vcs_contract_results.json): small reproduced counterexamples, not native benchmarks.

The project still uses the AVC name. Tavi is a provisional naming idea, not an approved rename.

## Existing prototype checks

    cargo test
    cargo fmt --check
    cargo clippy --all-targets --all-features -- -D warnings

The separate research probes can be run with Python 3.11+ and Git:

    python docs/experiments/vcs_contract_probes.py --output /tmp/avc-probes.json

## License

MIT. See [LICENSE](LICENSE).
