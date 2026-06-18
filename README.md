# avc

> **⚠️ WORK IN PROGRESS** — This project is under active development. APIs and data formats may change without notice. Do not use in production or with important repositories yet.

**Agent-Native Git-Compatible VCS**

A version control system designed for AI agent workflows, built on top of Git. Provides automatic snapshots, non-destructive undo, and a unified timeline — all while remaining fully compatible with standard Git tools.

## Features

- **Zero-ceremony safety** — Auto-snapshots on every command. Never lose work.
- **Non-destructive undo** — Step back through your timeline without moving Git HEAD or polluting `git log`.
- **Unified timeline** — Auto-snapshots and named changes live on the same timeline.
- **Agent-native** — `--json` output on all commands for easy integration with AI agents.
- **Git-compatible** — Works alongside normal Git operations. Hidden refs don't appear in `git log`, `git branch`, etc.
- **Privacy-first** — No full prompts stored by default. Only metadata.

## Installation

```bash
cargo install avc
```

Or build from source:

```bash
git clone https://github.com/nijaru/avc.git
cd avc
cargo build --release
```

## Quick Start

```bash
# Initialize avc in a Git repository
avc init

# Make some changes to your files
echo "hello" > file.txt

# Name your current work
avc change "initial implementation"

# Continue working
echo "world" >> file.txt

# Step back to the previous state
avc undo

# Jump to a specific point in time
avc restore <change-id>

# View your timeline
avc log

# Check current status
avc status
```

## Commands

| Command | Description |
|---------|-------------|
| `avc init` | Initialize avc in the current Git repository |
| `avc change <name>` | Name the current working directory state |
| `avc log` | Show unified timeline (auto-snapshots + named changes) |
| `avc undo` | Step back to the previous timeline point |
| `avc restore <id>` | Jump to a specific timeline point |
| `avc status` | Show current branch, HEAD, and last change |
| `avc doctor` | Run health checks |

### Common Flags

- `--json` — Output in JSON format (for agent consumption)
- `--changes` — Show only named changes in `log`
- `--limit N` — Limit number of entries in `log`
- `--clean` — Remove untracked files during `undo`/`restore`

## How It Works

avc stores snapshots as Git commits under hidden refs (`refs/agentvcs/*`). These refs are invisible to normal Git operations but can be accessed via avc commands.

**Auto-snapshots**: Every time you run an avc command, the working directory is automatically captured. If nothing has changed (detected via file modification times), the snapshot is skipped.

**Named changes**: When you run `avc change <name>`, a snapshot is created and recorded in a local SQLite database with the given name.

**Undo**: Steps back one point on the timeline. The working directory is restored without moving Git HEAD, so your normal Git history remains clean.

**Restore**: Jumps to a specific timeline point by ID prefix.

## Use Cases

- **AI Agent Workflows**: Agents can safely experiment with code changes, knowing they can always undo.
- **Iterative Development**: Try multiple approaches without fear of losing work.
- **Debugging**: Step back through your timeline to find when a bug was introduced.
- **Code Review**: Easily see what changed between named checkpoints.

## Architecture

- **Git objects** — Store file trees and commit metadata
- **SQLite** — Stores mutable workflow state (operations, changes, metadata)
- **Hidden refs** — `refs/agentvcs/auto/*` and `refs/agentvcs/changes/*`
- **Pre-push hook** — Prevents accidental push of hidden refs

## Requirements

- Git 2.20+
- Rust 1.75+ (for building from source)

## License

MIT OR Apache-2.0
