mod cli;
mod commands;
mod config;
mod git;
mod oplog;
mod output;
mod track;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Commands};

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Default to status if no command given
    let command = cli.command.unwrap_or(Commands::Status);

    let result = match command {
        Commands::Init => commands::init::run(cli.json),
        Commands::Save { message, amend } => {
            commands::save::run(message, amend, cli.json)
        }
        Commands::Undo => commands::undo::run(cli.json),
        Commands::Redo => commands::redo::run(cli.json),
        Commands::Log { limit, saves } => commands::log_cmd::run(limit, saves, cli.json),
        Commands::Status => commands::status::run(cli.json),
        Commands::Run { args } => commands::run::run(args, cli.json),
    };

    if let Err(e) = result {
        if cli.json {
            println!("{{\"error\": \"{}\"}}", e);
        } else {
            output::error(&format!("{:#}", e));
        }
        std::process::exit(1);
    }

    Ok(())
}
