# AVC

**Experimental native version control for people and coding agents.**

AVC is designed for everyday source editing on a laptop or VPS, independent agent workspaces, remote cloud development, short-lived containers/microVMs, and large CI/source-query workloads.

## Selected architecture (October 2026)

The v0 design uses **immutable content-addressed source States**, a separate immutable **Revision DAG**, and **stable logical ChangeIds** for editable history. The canonical source namespace uses deterministic persistent Merkle radix directory maps and includes ordinary files, executable mode, symlinks, empty directories, and typed unresolved conflicts.

The other core decisions fit this source authority:

- Private recovery is separate from deliberate commits and sharing. Explicit recovery includes eligible new files without tracking or authoring them. Ordinary checkpoints expire; work explicitly preserved for return stays protected until verified resume or deliberate discard.
- Serial work can preserve and resume source in one directory. Concurrent writers use distinct workspaces with independent source selection, recovery heads and materialization outcomes. Shared targets use conditional updates.
- Safe source reads, diffs, merges and conditional edits work by immutable IDs **without a checkout**; full repositories work offline, with optional native sparse and hosted clients.
- Conservative three-way merging preserves nested binary conflicts and separate typed provenance. Rebase creates new immutable revisions; undo and content revert are distinct.
- One SQLite authority owns local objects and roots; large payloads use bounded staged segment rows before short durable root transactions.
- Hosted publication is a later protocol with exact candidate validation, not an initial-core object or dependency.
- Git interoperability is a deferred boundary interface, not a required Git backend. Conversion is a likely starting point; the exact adapter shape remains open. Unsupported/lossy conversion must be explicit; native export remains the full-fidelity custody path. Agent runtimes and hosted services are not native requirements.

**The selected architecture and proposed v0 byte format are specified and undergoing independent qualification**, not proven globally optimal or released as stable. The [authoritative specification](docs/design.md) includes typed BLAKE3 IDs, a canonical nibble-radix Merkle source map, conservative bounded merge/conflict semantics, exact revision evolution, guarded workspace recovery, GC-safe staged admission/backup, typed Transform/Merge/Observation claims with separately retained private evidence, and future sparse/publication boundaries. Python, Node and an isolated Rust codec fixture now agree on the pinned examples and bounded differential corpus. Bounded typed-operation checks now exercise replay and evidence separation; complete merge/admission, recovery/durability qualification and representative layout/merge evidence remain **before format freeze**. Algorithms and byte constants remain candidates to qualify, not product requirements. Earlier changeset-first and causal graph candidates remain documented as rejected defaults.

## User experience

The [workflow contract](docs/ux.md) separates private protection from commit selection, addresses unfinished work before its first commit, and proposes same-directory interruption/return with optional concurrent directories. It also covers history-only undo versus file recovery, conflicts, review scope and sharing/backup disclosure. **The interaction model and command spellings are candidates, not an implemented or usability-validated CLI.** The [research review](docs/research.md#15-product-and-user-experience-evidence--october-28-2026) distinguishes user evidence, observed tool behavior and design inferences; the [local-work walkthrough](docs/research.md#16-local-work-addressing-and-resume-qualification--october-28-2026) refines creation, ownership and closure without claiming usability validation. Navigation, edit transfer/stack interactions and the Git adoption journey still need qualification; source-storage feasibility does not settle them.

## Current status

**The native engine has not been implemented.** The Rust 0.0.1 code is a legacy Git-command wrapper with a separate JSON operation log. It does not provide the native durability, recovery, conflict, workspace, or sync guarantees described above. Do not entrust it with the sole copy of important work.

- [Architecture](docs/design.md): **single authoritative v0 specification** for canonical objects, directory representation, conflicts, recovery, durability, APIs and publication.
- [Research](docs/research.md): evidence, rejected models, and adversarial design audit with counterexamples.
- [Qualification roadmap](docs/roadmap.md): independent golden-vector, merge, durability, portability and full-workflow verification gates.
- [Python reference fixtures](docs/experiments/test_codec.py), [independent JavaScript verifier](docs/experiments/codec_crosscheck.mjs), [Rust codec fixture](docs/experiments/codec_rust/main.rs) and [pinned object vectors](docs/experiments/codec_vectors.json): executable partial Milestone-0 qualification, **not** a frozen/production codec.
- [Source-model oracle](docs/experiments/source_models_oracle.py), [sparse text-merge reference](docs/experiments/text_merge_reference.py), [authority interleaving model](docs/experiments/authority_model.py), [bounded work-lifecycle model](docs/experiments/work_lifecycle_model.py) and [contract probes](docs/experiments/vcs_contract_probes.py): bounded research/verification artifacts.
- [Selective-authoring reference](docs/experiments/selective_authoring_reference.py) and [contract tests](docs/experiments/test_selective_authoring.py): canonical-object checks for subset commits, recorded-base recovery and private metadata separation, not a native authoring implementation.
- [Typed-operation reference](docs/experiments/operation_reference.py) and [contract tests](docs/experiments/test_operations.py): canonical-record replay, protected evidence and observation obligations, not complete native admission or move-aware merging.
- [SQLite boundary probes](docs/experiments/test_sqlite_custody.py) and [map-layout study](docs/experiments/map_layout_study.py): physical transaction checks and logical byte/object comparisons, **not** a native store, power-loss proof or engine benchmark.

The project remains named AVC. Tavi and Delos were exploratory names only.

## Legacy prototype checks

    cargo test
    cargo fmt --check
    cargo clippy --all-targets --all-features -- -D warnings

## License

MIT. See [LICENSE](LICENSE).
