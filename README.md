# AVC

Version control for independent work and deliberate integration.

AVC is being redesigned as a **local-first, change-oriented VCS for humans and agents**, interoperating with existing Git projects. The intended system protects unfinished work, gives concurrent workers isolated workspaces, and makes the transition from proposed changes to shared history explicit and verifiable.

## Implementation status

**The redesign is a specification, not an implemented feature set.** The current Rust package is the earlier `avc` 0.0.1 prototype: it invokes Git, creates automatic commits, and records a separate JSON operation log. It does not implement the transactional storage, recovery guarantees, workspace isolation, or validated integration described below. Keep independent backups; do not rely on the prototype to protect irreplaceable work.

The implementation direction is to reuse **Jujutsu's Rust history engine and Git object storage**, with AVC-owned transactional coordination and bounded recovery. It is not a continuation of the automatic-commit/squash command loop, and it does not start by inventing a new object-store format.

## Intended experience

A person works on a named change in an ordinary directory. Work is recoverable without turning every edit into a public commit. Splitting, amending, rebasing, and undoing work are normal operations.

A worker needing independence gets another workspace sharing immutable repository data, not another writer in the same mutable directory. Agents use the same operations through a versioned, noninteractive API with explicit revision and concurrency preconditions.

Local recovery, private synchronization, review proposals, and integration are different actions. Shared integration validates an exact candidate against the current target and policy. Existing Git hosting remains usable; an optional managed service adds guarantees that an ordinary Git push cannot provide.

These are design goals. See the roadmap for implementation boundaries and acceptance criteria.

## Design and implementation

- [Architecture and contracts](docs/design.md): accepted direction, state ownership, recovery, concurrency, interoperability, and integration semantics.
- [Research and alternatives](docs/research.md): primary-source evidence, lessons from existing systems, rejected alternatives, and the **Tavi** naming recommendation.
- [Implementation roadmap](docs/roadmap.md): working milestones, compatibility qualification, failure tests, and workload comparisons.

The repository, package, binary, and `.avc` directory retain their current names. Tavi is a recommendation, not an applied rename.

## Development

The existing crate's checks are:

```sh
cargo test
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```

Passing these checks does not establish the redesigned system's guarantees. Those need the end-to-end acceptance tests in the roadmap.

## License

MIT. See [LICENSE](LICENSE).
