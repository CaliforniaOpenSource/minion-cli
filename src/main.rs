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
    /// Deploy the current project. Use -y to skip prompts and use .minion defaults
    Deploy {
        /// Use defaults from .minion config 
        #[arg(short, long)]
        yes: bool,
    },
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
        Commands::Deploy { yes } => {
            DeployCommand::new().execute(yes)?;
        }
    }

    Ok(())
}
