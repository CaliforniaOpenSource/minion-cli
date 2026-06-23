mod base64;
mod command;
mod fs_atomic;
mod http;
mod json;
mod model;
mod paths;
mod provision;
mod reconcile;
mod store;
#[cfg(test)]
mod test_support;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};

use command::SystemCommandRunner;
use paths::HubPaths;

const INTERFACE: &str = "minion0";
const HUB_VPN_IP: Ipv4Addr = Ipv4Addr::new(10, 42, 42, 1);
const VPN_CIDR: &str = "10.42.42.0/24";
const WG_LISTEN_PORT: u16 = 51820;
const API_LISTEN_ADDR: &str = "10.42.42.1:4242";
const COREDNS_VERSION: &str = "1.12.4";
const TEST_PRIVATE_KEY: &str = "k5bSV80vBajcVrgkjDT6Tq+OPqnVUjyTfsZnTvPKjAk=";
const TEST_PUBLIC_KEY: &str = "MaBtQgZi76tAZmxb8ujzWsb5yAJlZ38JMf6GikKtAS0=";

#[derive(Parser)]
#[command(author, version, about = "Hub companion executable for Minion", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Install and configure the private hub node
    Init(InitArgs),
    /// Serve the private hub HTTP API
    Serve(ServeArgs),
}

#[derive(Args)]
struct InitArgs {
    /// Test-only root for generated system files
    #[arg(long, hide = true)]
    config_root: Option<PathBuf>,

    /// Test-only mode that skips apt, systemctl, sysctl, and wg commands
    #[arg(long, hide = true)]
    skip_system: bool,
}

#[derive(Args)]
struct ServeArgs {
    /// Test-only root for generated system files
    #[arg(long, hide = true)]
    config_root: Option<PathBuf>,

    /// HTTP listen address. The default is private WireGuard only.
    #[arg(long, hide = true, default_value = API_LISTEN_ADDR)]
    listen: SocketAddr,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init(args) => {
            let paths = paths_from_arg(args.config_root.as_deref());
            let runner = SystemCommandRunner::new(args.skip_system);
            provision::init_hub(&paths, &runner)?;
        }
        Commands::Serve(args) => {
            let paths = paths_from_arg(args.config_root.as_deref());
            http::serve(paths, args.listen)?;
        }
    }

    Ok(())
}

fn paths_from_arg(root: Option<&Path>) -> HubPaths {
    match root {
        Some(root) => HubPaths::under_root(root),
        None => HubPaths::default(),
    }
}

fn wg_quick_unit() -> String {
    format!("wg-quick@{}", INTERFACE)
}
