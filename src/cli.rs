use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "avc", version, about = "Agent-native Git-compatible VCS")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// JSON output for machine consumption
    #[arg(long, global = true)]
    pub json: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialize avc in a git repository
    Init,

    /// Squash auto-commits into a clean commit
    Save {
        /// Commit message (can be specified multiple times for paragraphs)
        #[arg(short, long = "message")]
        message: Vec<String>,

        /// Squash into the last save instead of creating a new one
        #[arg(long)]
        amend: bool,
    },

    /// Step back one operation (non-destructive)
    Undo,

    /// Step forward one operation (non-destructive)
    Redo,

    /// View the operation timeline
    Log {
        /// Limit number of entries
        #[arg(short, long)]
        limit: Option<usize>,

        /// Show only saved commits
        #[arg(long)]
        saves: bool,
    },

    /// Show changes since last save
    Status,

    /// Wrap a command with before/after snapshots
    #[command(allow_external_subcommands = true)]
    Run {
        /// Command and arguments to run
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Watch for file changes and auto-commit periodically
    Watch {
        /// Auto-commit interval in seconds
        #[arg(short, long, default_value = "2")]
        interval: u64,
    },
}
