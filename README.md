# AVC

**A native, local-first version control system designed for developers and coding agents.**

AVC's planned core combines immutable persistent source trees, editable history, reliable offline custody and recovery, fast selective native clones, conditional source APIs without checkout, and Git interoperability **without storing native history in Git**.

## Status — November 4, 2026

**Architecture defined; implementation intentionally reset.** No functioning native VCS, safe source custody, stable wire format, clone/import, backup or agent API exists yet. The active repository contains the **implementation-ready design contracts**, not an experimental old codec masquerading as the new system.

The former Rust prototype, independent Python/Node/Rust fixtures, old format vectors and CI were retired because they implemented a **different unreleased architecture** (4 KiB map leaves, old State/provenance objects, nested conflicts and whole-map rebuilding). Their source, original measurements and reports are preserved in [the pinned pre-cleanup commit](https://github.com/nijaru/avc/tree/af66a63898f383b596e4828199dc625a80e28a72). Consult it only as historical evidence, **not** a live test suite or authoritative byte format. Existing user Git repositories and legacy `.avc` recovery data must never be automatically overwritten or discarded.

**Next step:** start a fresh Rust library and CLI on the local machine, implement the selected canonical source engine, then qualify durable local operations before storing important user source. No further general architecture review is required by default.

## Design ownership and reading order

1. [Architecture](docs/design.md): source ownership, invariants, responsibilities and selected system boundaries.
2. [Canonical format and merge](docs/format.md): exact BLAKE3 object IDs, binary encoding, persistent radix directory maps, revisions, typed signed conflicts, path/merge semantics.
3. [Local store and recovery](docs/storage.md): SQLite authority, immutable compressed segments, crash ordering, GC, working-directory observation, guarded materialization, operation log, backup/restore.
4. [Native synchronization](docs/sync.md): tip-first/partial clones, Merkle proofs, object promises, source APIs, conditional remote publication.
5. [Git integration](docs/git.md): complete-tip native import, foreign partial views, ancestry mapping, export/push and explicit fidelity boundaries.
6. [User workflows](docs/ux.md): selected first-implementation commands, source/history editing, checkpoints, workspaces, change stacks and agent use.
7. [Execution plan](docs/roadmap.md): local implementation stages, qualification and release gates.
8. [Adversarial decisions and evidence](docs/research.md): why this model was selected over Git-backed, Scion, Pijul, GitButler and earlier AVC alternatives.

The design is **complete as an implementation contract**, but has not passed independent codec vectors, crash tests or real workload benchmarks. A concrete failed invariant or significant measured cost can justify a coherent revision **before v1 format freeze**. Do not preserve wrong code or unvalidated constants simply because they appeared in an earlier chat or experiment.

## Primary design choices

- **Source model:** typed BLAKE3-identified Merkle roots, path-copy persistent radix maps, immutable revisions and stable logical ChangeIds.
- **Conflict handling:** normalized signed exact-entry expressions and deterministic three-way merge; unresolved source is represented as data, not clean marker text.
- **Local custody:** SQLite WAL/FULL metadata plus staged immutable payload segments, independent closure-based backup and recovery.
- **Clones and agents:** current source first, bounded history horizon, on-demand native objects, optional sparse/no-checkout clients and conditional path proofs.
- **Git adoption:** permanent native import/export and remote synchronization, with a separate foreign Git partial mode for unfetched objects. **Later Git smart-protocol serving is planned from the same native store and authoritative refs**, not through a second Git-backed AVC engine.
- **History UX:** scoped local operation undo, private checkpoints, separate uncommitted work, optional composed multi-change stacks and `absorb`.

## Safety and license

AVC is not ready to own irreplaceable data. Keep complete independent Git/other backups until source admission, power-loss recovery, GC, and exported independent restores have been proven on the real native CLI.

MIT. See [LICENSE](LICENSE).
