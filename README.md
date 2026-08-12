# AVC

**Atomic Version Control**

> **Early development:** AVC is being rebuilt around its own object model and storage rather than Git. Its formats and commands are not stable yet, so keep independent backups until the durability work is complete.

AVC is designed around **automatic recovery, intentional history, and atomic publication**. It separates private filesystem recovery from named development changes and shared repository events.

## Model

| Concept | Purpose | Default visibility |
| --- | --- | --- |
| **Savepoint** | Automatically capture a recoverable filesystem state | Private and retention-bounded |
| **Change** | Give a stable identity and description to a logical unit of work | Local or draft |
| **Revision** | Record one immutable version of a change | Local or draft |
| **Publication** | Atomically update shared project state and trigger integrations | Shared |

Automatic savepointing observes the working copy; it does not rewrite editor buffers or files. A commit-like action finishes and names the current change. Publishing selected changes is a separate operation, so private recovery does not inherently create branches, CI runs, webhooks, notifications, or indexing work.

## Status

The accepted direction is a journaled snapshot VCS with:

- immutable content-addressed objects;
- stable change identities distinct from revision identities;
- a separate operation/view graph for undo and recovery;
- bounded automatic savepoints;
- first-class workspaces and conflicts;
- one append-only durable write path;
- derived, rebuildable indexes;
- quiet draft synchronization; and
- compare-and-swap publication.

The existing `src/` tree is an earlier Git-backed prototype. It remains only as implementation evidence while the Git-independent implementation replaces it; it does not implement the architecture above.

See [`docs/design.md`](docs/design.md) for the architecture, correctness invariants, research sources, and implementation roadmap.

## First working milestone

The first implementation milestone is intentionally focused:

```text
avc init
avc checkpoint [name]
avc status
avc log
avc restore <id>
```

It will replace the Git-backed command loop with AVC whole-file objects, deterministic flat snapshots, and durable local state in the existing Rust crate. This is foundation code for the real tool, not a disposable spike.

From there, AVC will grow through working releases that add:

1. bounded automatic savepoints and stable named changes;
2. operation-log undo, restore, and history rewriting;
3. first-class workspaces, deterministic merge, and conflict-bearing revisions;
4. AVC draft synchronization and atomic publication; and
5. scalable indexes, chunking, and lazy materialization where the working tool needs them.

Current checks:

```bash
cargo test
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```

## License

MIT — see [`LICENSE`](LICENSE).
