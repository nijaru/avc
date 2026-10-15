# AVC

**Experimental native version control for people and coding agents.**

AVC is designed for everyday source editing on a laptop or VPS, independent agent workspaces, remote cloud development, short-lived containers/microVMs, and large CI/source-query workloads.

## Selected architecture (October 2026)

The v0 design uses **immutable content-addressed source States**, a separate immutable **Revision DAG**, and **stable logical ChangeIds** for editable history. The canonical source namespace uses deterministic persistent Merkle radix directory maps and includes ordinary files, executable mode, symlinks, empty directories, and typed unresolved conflicts.

The other core decisions fit this source authority:

- Private, bounded recovery captures are separate from permanent authored history and shared publication.
- Distinct workspaces own distinct writable directories, source selection, recovery heads and materialization outcomes. Shared targets use conditional updates.
- Safe source reads, diffs, merges and conditional edits work by immutable IDs **without a checkout**; full repositories work offline, with optional native sparse and hosted clients.
- Conservative three-way merging preserves typed conflicts and separate origin provenance. Rebase creates new immutable revisions; undo and content revert are distinct.
- SQLite owns short local root transactions; immutable large objects have a protected staging and garbage-collection protocol.
- Shared publication validates an exact candidate under an explicit policy.
- Git compatibility, Git object formats, Git protocol, agent runtimes and hosted services are **not native requirements**. Any adapter is a separate later decision.

**This is a selected architecture, not an experimentally demonstrated global optimum.** Earlier changeset-first and causal graph proposals have been evaluated and rejected as the default; their design arguments remain in the research record. Engineering qualification may tune representation parameters or expose a concrete correctness defect, but the project is not waiting for another broad architecture selection.

## Current status

**The native engine has not been implemented.** The Rust 0.0.1 code is a legacy Git-command wrapper with a separate JSON operation log. It does not provide the native durability, recovery, conflict, workspace, or sync guarantees described above. Do not entrust it with the sole copy of important work.

- [Architecture](docs/design.md): selected data model, canonical tree structure, source/merge semantics, recovery, storage, APIs, and publication.
- [Research](docs/research.md): comparative evidence and reasons for rejecting other canonical models.
- [Qualification roadmap](docs/roadmap.md): correctness, durability, portability, user-workflow and performance gates.
- [Source-model oracle](docs/experiments/source_models_oracle.py) and [contract probes](docs/experiments/vcs_contract_probes.py): narrow research artifacts, **not** native benchmarks.

The project remains named AVC. Tavi and Delos were exploratory names only.

## Legacy prototype checks

    cargo test
    cargo fmt --check
    cargo clippy --all-targets --all-features -- -D warnings

## License

MIT. See [LICENSE](LICENSE).
