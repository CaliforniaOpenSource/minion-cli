use clap::{Args, Parser, Subcommand};
mod commands;
mod utils;

use commands::{
    ControlAction, ControlCommand, DeployCommand, DeployOptions, InitCommand, SetupCommand,
};
use utils::AppConfigOverrides;

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

        /// Fail instead of prompting when required configuration is missing
        #[arg(long)]
        ci: bool,

        #[command(flatten)]
        common: CommonArgs,

        /// App URL/domain, or comma-separated URLs
        #[arg(long)]
        url: Option<String>,

        /// Port the app listens on inside the container
        #[arg(long)]
        port: Option<String>,

        /// Volume mappings as local:remote pairs, comma separated
        #[arg(long)]
        volumes: Option<String>,

        /// Docker build platform
        #[arg(long)]
        docker_platform: Option<String>,
    },
    /// Show container status and recent logs for the current app
    Status {
        #[command(flatten)]
        common: CommonArgs,
    },
    /// Show docker compose ps output for the current app
    Ps {
        #[command(flatten)]
        common: CommonArgs,
    },
    /// Show logs for the current app
    Logs {
        #[command(flatten)]
        common: CommonArgs,

        /// Follow log output until interrupted
        #[arg(short, long)]
        follow: bool,

        /// Number of log lines to show
        #[arg(long, default_value_t = 100)]
        tail: u16,
    },
    /// Restart the current app
    Restart {
        #[command(flatten)]
        common: CommonArgs,
    },
    /// Stop the current app
    Stop {
        #[command(flatten)]
        common: CommonArgs,
    },
    /// Start the current app
    Start {
        #[command(flatten)]
        common: CommonArgs,
    },
    /// Check server prerequisites and app deployment state
    Doctor {
        #[command(flatten)]
        common: CommonArgs,
    },
}

#[derive(Args, Debug, Clone, Default)]
struct CommonArgs {
    /// VPS hostname or IP address
    #[arg(long)]
    host: Option<String>,

    /// Minion app name
    #[arg(long = "app")]
    app_name: Option<String>,

    /// SSH user for the VPS
    #[arg(long)]
    ssh_user: Option<String>,

    /// Path to an SSH private key
    #[arg(long)]
    ssh_key_path: Option<String>,

    /// SSH private key content
    #[arg(long)]
    ssh_private_key: Option<String>,

    /// SSH password
    #[arg(long)]
    ssh_password: Option<String>,

    /// SSH key passphrase
    #[arg(long)]
    ssh_passphrase: Option<String>,
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
        Commands::Deploy {
            yes,
            ci,
            common,
            url,
            port,
            volumes,
            docker_platform,
        } => {
            let mut overrides = overrides_from_common(common);
            overrides.app_url = url;
            overrides.app_port = port;
            overrides.app_volumes = volumes;
            overrides.docker_platform = docker_platform;

            DeployCommand::new().execute(DeployOptions { yes, ci, overrides })?;
        }
        Commands::Status { common } => {
            ControlCommand::new().execute(ControlAction::Status, overrides_from_common(common))?;
        }
        Commands::Ps { common } => {
            ControlCommand::new().execute(ControlAction::Ps, overrides_from_common(common))?;
        }
        Commands::Logs {
            common,
            follow,
            tail,
        } => {
            ControlCommand::new().execute(
                ControlAction::Logs { follow, tail },
                overrides_from_common(common),
            )?;
        }
        Commands::Restart { common } => {
            ControlCommand::new().execute(ControlAction::Restart, overrides_from_common(common))?;
        }
        Commands::Stop { common } => {
            ControlCommand::new().execute(ControlAction::Stop, overrides_from_common(common))?;
        }
        Commands::Start { common } => {
            ControlCommand::new().execute(ControlAction::Start, overrides_from_common(common))?;
        }
        Commands::Doctor { common } => {
            ControlCommand::new().execute(ControlAction::Doctor, overrides_from_common(common))?;
        }
    }

    Ok(())
}

fn overrides_from_common(common: CommonArgs) -> AppConfigOverrides {
    AppConfigOverrides {
        host: common.host,
        app_name: common.app_name,
        ssh_user: common.ssh_user,
        ssh_key_path: common.ssh_key_path,
        ssh_private_key: common.ssh_private_key,
        ssh_password: common.ssh_password,
        ssh_passphrase: common.ssh_passphrase,
        ..Default::default()
    }
}
