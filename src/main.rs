use clap::{Parser, Subcommand};
mod commands;
mod utils;

use commands::{SetupCommand, InitCommand, DeployCommand};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Sets up the minion environment on the VPS
    Setup,
    /// Initialize a new minion project
    Init,
    /// Deploy the current project
    Deploy,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Setup => {
            SetupCommand::new().execute()?;
        }
        Commands::Init => {
            InitCommand::new().execute()?;
        }
        Commands::Deploy => {
            DeployCommand::new().execute()?;
        }
    }

    Ok(())
}
