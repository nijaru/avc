use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "avc", version, about = "Agent-Native Git-Compatible VCS")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Output in JSON format
    #[arg(long, global = true)]
    pub json: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialize avc in this repository
    Init,

    /// Name the current work as a change
    Change {
        /// Change title (positional argument)
        name: Option<String>,

        /// Message flag (alias for positional)
        #[arg(short = 'm', long = "message")]
        message: Option<String>,
    },

    /// Show the unified timeline
    Log {
        /// Number of entries to show
        #[arg(long, default_value = "20")]
        limit: u32,

        /// Show only named changes
        #[arg(long)]
        changes: bool,
    },

    /// Step back to previous timeline point
    Undo {
        /// Also remove untracked files
        #[arg(long)]
        clean: bool,
    },

    /// Jump to a specific timeline point
    Restore {
        /// Change or operation ID (prefix match)
        id: String,

        /// Also remove untracked files
        #[arg(long)]
        clean: bool,
    },

    /// Show working tree status
    Status,

    /// Run health check
    Doctor,
}
