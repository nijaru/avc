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

**The selected architecture and proposed v0 byte format are specified and undergoing independent qualification**, not proven globally optimal or released as stable. The [authoritative specification](docs/design.md) includes typed BLAKE3 IDs, a canonical nibble-radix Merkle source map, conservative bounded merge/conflict semantics, exact revision evolution, guarded workspace recovery, GC-safe staged admission/backup, private sparse security boundaries and typed publication candidates with target-generation CAS. The next work is independent format vectors and adversarial fault/merge verification, **not** another general VCS-model search. Earlier changeset-first and causal graph candidates remain documented as rejected defaults.

## Current status

**The native engine has not been implemented.** The Rust 0.0.1 code is a legacy Git-command wrapper with a separate JSON operation log. It does not provide the native durability, recovery, conflict, workspace, or sync guarantees described above. Do not entrust it with the sole copy of important work.

- [Architecture](docs/design.md): **single authoritative v0 specification** for canonical objects, directory representation, conflicts, recovery, durability, APIs and publication.
- [Research](docs/research.md): evidence, rejected models, and adversarial design audit with counterexamples.
- [Qualification roadmap](docs/roadmap.md): independent golden-vector, merge, durability, portability and full-workflow verification gates.
- [Codec reference fixtures](docs/experiments/test_codec.py) and [pinned object vectors](docs/experiments/codec_vectors.json): executable partial Milestone-0 qualification, **not** a frozen/production codec.
- [Source-model oracle](docs/experiments/source_models_oracle.py) and [contract probes](docs/experiments/vcs_contract_probes.py): narrow research artifacts, **not** native benchmarks.

The project remains named AVC. Tavi and Delos were exploratory names only.

## Legacy prototype checks

    cargo test
    cargo fmt --check
    cargo clippy --all-targets --all-features -- -D warnings

## License

MIT. See [LICENSE](LICENSE).
