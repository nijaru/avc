mod cli;
mod commands;
mod config;
mod db;
mod git;
mod id;
mod snapshot;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Commands};

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Default to status if no command given
    let command = cli.command.unwrap_or(Commands::Status);

    let result = match command {
        Commands::Init => commands::init::run(cli.json),
        Commands::Change { name, message } => {
            let title = name.or(message);
            commands::change::run(title, cli.json)
        }
        Commands::Log { limit, changes } => commands::log::run(limit, changes, cli.json),
        Commands::Undo { clean } => commands::undo::run(clean, cli.json),
        Commands::Restore { id, clean } => commands::restore::run(&id, clean, cli.json),
        Commands::Status => commands::status::run(cli.json),
        Commands::Doctor => commands::doctor::run(cli.json),
    };

    if let Err(e) = result {
        if cli.json {
            println!("{{\"error\": \"{}\"}}", e);
        } else {
            eprintln!("Error: {:#}", e);
        }
        std::process::exit(1);
    }

    Ok(())
}
