# avc

> **⚠️ WORK IN PROGRESS** — This project is under active development. APIs and data formats may change without notice. Do not use in production or with important repositories yet.

**Agent-Native Git-Compatible VCS**

A version control layer designed for AI agent workflows, built on top of Git. Auto-commit, easy squash, non-destructive undo — all while remaining fully compatible with standard Git tools.

## Features

- **Zero-ceremony safety** — Auto-commits on every command. Never lose work.
- **Non-destructive undo** — Step back through your operation history, not destructive reverts.
- **No staging area** — Everything gets committed. No `git add`, no index management.
- **Agent-native** — `--json` output on all commands for easy integration with AI agents.
- **Git-compatible** — Commits are visible in `git log`. No hidden refs, no magic. Works alongside normal Git operations.
- **Transparent** — All state is in `.avc/oplog` (JSONL) and `.avc/config` (YAML). No databases, no hidden state.

## Installation

Build from source:

```bash
git clone https://github.com/nijaru/avc.git
cd avc
cargo install --path .
```

## Quick Start

```bash
# Initialize avc in a Git repository
avc init

# Make some changes — they're auto-committed on the next avc command
echo "hello" > file.txt

# Squash auto-commits into a clean save
avc save -m "add file.txt"

# View your operation timeline
avc log

# Step back (non-destructive)
avc undo

# Step forward
avc redo

# Wrap a command with before/after snapshots
avc run -- cargo test

# Or start the file watcher for continuous auto-commit
avc watch --interval 2
```

## Commands

| Command | Description |
|---------|-------------|
| `avc init` | Initialize avc in the current Git repository |
| `avc save [-m MSG] [--amend]` | Squash auto-commits into a clean commit |
| `avc undo` | Step back one operation (non-destructive) |
| `avc redo` | Step forward one operation (non-destructive) |
| `avc log [--saves] [--limit N]` | View the operation timeline |
| `avc status` | Show branch, last save, and uncommitted changes |
| `avc run -- <cmd>` | Wrap a command with before/after snapshots |
| `avc watch [--interval N]` | Watch for file changes and auto-commit |

### Flags

- `--json` — Output in JSON format (for agent consumption)
- `--saves` — Show only saves (not auto-commits) in `log`
- `--limit N` — Limit number of entries in `log`
- `--amend` — Squash into the last save instead of creating a new one

## How It Works

avc auto-commits your working tree on every command and maintains an operation log (oplog) for undo/redo.

**Auto-commits**: Every `avc` command auto-commits dirty files as `[avc:auto]` commits. These are real Git commits, visible in `git log`.

**Save**: `avc save` squashes consecutive auto-commits into a single clean commit with your message. The oplog records which auto-commits were squashed.

**Undo/Redo**: Steps through the oplog using `git reset`. Non-destructive — all operations are recorded and can be redone.

**Watch**: `avc watch` polls the working tree for changes and auto-commits at a configurable interval (default 2 seconds). Useful for background safety.

**Run**: `avc run -- <cmd>` snapshots before and after running a command, then records the result in the oplog.

## State

All avc state lives in `.avc/`:

| File | Format | Purpose |
|------|--------|---------|
| `.avc/oplog` | JSONL | Append-only operation log |
| `.avc/config` | YAML | Configuration |

The `.avc/` directory is added to `.gitignore` by `avc init`.

## Requirements

- Git 2.20+
- Rust 1.85+ (edition 2024)

## License

MIT — see [LICENSE](LICENSE).
