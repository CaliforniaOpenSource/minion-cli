use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use std::collections::{BTreeMap, HashMap};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

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
            init_hub(&paths, &runner)?;
        }
        Commands::Serve(args) => {
            let paths = paths_from_arg(args.config_root.as_deref());
            serve(paths, args.listen)?;
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

#[derive(Debug, Clone)]
struct HubPaths {
    wg_config: PathBuf,
    wg_public_key: PathBuf,
    coredns_corefile: PathBuf,
    coredns_hosts: PathBuf,
    coredns_bin: PathBuf,
    coredns_service: PathBuf,
    hub_service: PathBuf,
    sysctl_file: PathBuf,
    install_bin: PathBuf,
}

impl HubPaths {
    fn default() -> Self {
        Self {
            wg_config: PathBuf::from("/etc/wireguard/minion0.conf"),
            wg_public_key: PathBuf::from("/etc/wireguard/minion0.pub"),
            coredns_corefile: PathBuf::from("/etc/coredns/Corefile"),
            coredns_hosts: PathBuf::from("/etc/coredns/minion.hosts"),
            coredns_bin: PathBuf::from("/usr/local/bin/coredns"),
            coredns_service: PathBuf::from("/etc/systemd/system/coredns.service"),
            hub_service: PathBuf::from("/etc/systemd/system/minion-hub.service"),
            sysctl_file: PathBuf::from("/etc/sysctl.d/99-minion-hub.conf"),
            install_bin: PathBuf::from("/usr/local/bin/minion-hub"),
        }
    }

    fn under_root(root: &Path) -> Self {
        Self {
            wg_config: root.join("etc/wireguard/minion0.conf"),
            wg_public_key: root.join("etc/wireguard/minion0.pub"),
            coredns_corefile: root.join("etc/coredns/Corefile"),
            coredns_hosts: root.join("etc/coredns/minion.hosts"),
            coredns_bin: root.join("usr/local/bin/coredns"),
            coredns_service: root.join("etc/systemd/system/coredns.service"),
            hub_service: root.join("etc/systemd/system/minion-hub.service"),
            sysctl_file: root.join("etc/sysctl.d/99-minion-hub.conf"),
            install_bin: root.join("usr/local/bin/minion-hub"),
        }
    }
}

trait CommandRunner {
    fn enabled(&self) -> bool;
    fn run(&self, program: &str, args: &[&str]) -> Result<String>;
    fn run_with_stdin(&self, program: &str, args: &[&str], stdin: &str) -> Result<String>;
}

struct SystemCommandRunner {
    skip_system: bool,
}

impl SystemCommandRunner {
    fn new(skip_system: bool) -> Self {
        Self { skip_system }
    }
}

impl CommandRunner for SystemCommandRunner {
    fn enabled(&self) -> bool {
        !self.skip_system
    }

    fn run(&self, program: &str, args: &[&str]) -> Result<String> {
        if self.skip_system {
            return Ok(String::new());
        }

        let output = Command::new(program)
            .args(args)
            .output()
            .with_context(|| format!("failed to run {}", program))?;
        command_output(program, args, output)
    }

    fn run_with_stdin(&self, program: &str, args: &[&str], stdin: &str) -> Result<String> {
        if self.skip_system {
            return Ok(String::new());
        }

        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to run {}", program))?;

        child
            .stdin
            .as_mut()
            .context("failed to open command stdin")?
            .write_all(stdin.as_bytes())?;

        let output = child.wait_with_output()?;
        command_output(program, args, output)
    }
}

fn command_output(program: &str, args: &[&str], output: std::process::Output) -> Result<String> {
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        bail!(
            "{} {} failed: {}{}",
            program,
            args.join(" "),
            stdout,
            stderr
        );
    }
    Ok(stdout)
}

fn init_hub(paths: &HubPaths, runner: &dyn CommandRunner) -> Result<()> {
    ensure_root()?;
    install_packages(runner)?;

    let private_key = ensure_hub_keys(paths, runner)?;
    ensure_wireguard_config(paths, &private_key)?;
    ensure_ip_forwarding(paths, runner)?;
    ensure_coredns_files(paths)?;
    ensure_coredns_service(paths, runner)?;
    install_self_binary(paths)?;
    ensure_hub_service(paths)?;

    if runner.enabled() {
        runner.run("systemctl", &["daemon-reload"])?;
        runner.run("systemctl", &["enable", "--now", "wg-quick@minion0"])?;
        runner.run("systemctl", &["enable", "--now", "coredns"])?;
        runner.run("systemctl", &["enable", "--now", "minion-hub"])?;
    }

    println!("minion-hub initialized");
    println!("hub VPN IP: {}", HUB_VPN_IP);
    println!("API listen address: {}", API_LISTEN_ADDR);
    Ok(())
}

fn ensure_root() -> Result<()> {
    let output = Command::new("id")
        .arg("-u")
        .output()
        .context("failed to determine current user")?;
    if !output.status.success() {
        bail!(
            "failed to determine current user: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    if String::from_utf8_lossy(&output.stdout).trim() != "0" {
        bail!("minion-hub init must be run as root");
    }
    Ok(())
}

fn install_packages(runner: &dyn CommandRunner) -> Result<()> {
    if !runner.enabled() {
        return Ok(());
    }

    runner.run("apt-get", &["update"])?;
    runner.run(
        "apt-get",
        &[
            "install",
            "-y",
            "wireguard",
            "wireguard-tools",
            "ca-certificates",
            "curl",
        ],
    )?;
    if apt_package_available(runner, "coredns") {
        runner.run("apt-get", &["install", "-y", "coredns"])?;
    } else {
        install_coredns_release(runner)?;
    }
    Ok(())
}

fn apt_package_available(runner: &dyn CommandRunner, package: &str) -> bool {
    runner.run("apt-cache", &["show", package]).is_ok()
}

fn install_coredns_release(runner: &dyn CommandRunner) -> Result<()> {
    if runner
        .run("test", &["-x", "/usr/local/bin/coredns"])
        .is_ok()
    {
        return Ok(());
    }

    let arch = runner.run("dpkg", &["--print-architecture"])?;
    let platform = match arch.trim() {
        "amd64" => "linux_amd64",
        "arm64" => "linux_arm64",
        other => bail!("unsupported CoreDNS architecture: {}", other),
    };
    let archive = format!("/tmp/coredns_{}_{}.tgz", COREDNS_VERSION, platform);
    let url = format!(
        "https://github.com/coredns/coredns/releases/download/v{0}/coredns_{0}_{1}.tgz",
        COREDNS_VERSION, platform
    );

    runner.run("curl", &["-fsSL", "-o", &archive, &url])?;
    runner.run("tar", &["-xzf", &archive, "-C", "/tmp", "coredns"])?;
    runner.run(
        "install",
        &["-m", "0755", "/tmp/coredns", "/usr/local/bin/coredns"],
    )?;
    let _ = runner.run("rm", &["-f", &archive, "/tmp/coredns"]);
    Ok(())
}

fn ensure_hub_keys(paths: &HubPaths, runner: &dyn CommandRunner) -> Result<String> {
    if paths.wg_config.exists() {
        let contents = fs::read_to_string(&paths.wg_config)?;
        validate_wireguard_interface(&contents)?;
        let config = parse_wireguard_config(&contents)?;
        ensure_public_key_file(paths, &config.private_key, runner)?;
        return Ok(config.private_key);
    }

    let (private_key, public_key) = generate_hub_keys(runner)?;
    write_atomic_string(
        &paths.wg_public_key,
        &format!("{}\n", public_key),
        0o644,
        |_| Ok(()),
    )?;
    Ok(private_key)
}

fn ensure_public_key_file(
    paths: &HubPaths,
    private_key: &str,
    runner: &dyn CommandRunner,
) -> Result<()> {
    if paths.wg_public_key.exists() {
        return Ok(());
    }

    let public_key = if runner.enabled() {
        runner
            .run_with_stdin("wg", &["pubkey"], &format!("{}\n", private_key))?
            .trim()
            .to_string()
    } else {
        TEST_PUBLIC_KEY.to_string()
    };
    write_atomic_string(
        &paths.wg_public_key,
        &format!("{}\n", public_key),
        0o644,
        |_| Ok(()),
    )
}

fn generate_hub_keys(runner: &dyn CommandRunner) -> Result<(String, String)> {
    if !runner.enabled() {
        return Ok((TEST_PRIVATE_KEY.to_string(), TEST_PUBLIC_KEY.to_string()));
    }

    let private_key = runner.run("wg", &["genkey"])?.trim().to_string();
    let public_key = runner
        .run_with_stdin("wg", &["pubkey"], &format!("{}\n", private_key))?
        .trim()
        .to_string();
    Ok((private_key, public_key))
}

fn ensure_wireguard_config(paths: &HubPaths, private_key: &str) -> Result<()> {
    if paths.wg_config.exists() {
        return Ok(());
    }

    let config = WgConfig {
        private_key: private_key.to_string(),
        peers: Vec::new(),
    };
    save_wireguard_config(paths, &config)
}

fn ensure_ip_forwarding(paths: &HubPaths, runner: &dyn CommandRunner) -> Result<()> {
    let changed = write_atomic_string_if_changed(
        &paths.sysctl_file,
        "net.ipv4.ip_forward = 1\n",
        0o644,
        |_| Ok(()),
    )?;
    if changed && runner.enabled() {
        runner.run("sysctl", &["--system"])?;
    }
    Ok(())
}

fn ensure_coredns_files(paths: &HubPaths) -> Result<()> {
    if paths.coredns_hosts.exists() {
        parse_hosts(&fs::read_to_string(&paths.coredns_hosts)?)?;
    } else {
        write_atomic_string(&paths.coredns_hosts, "", 0o644, |path| {
            parse_hosts(&fs::read_to_string(path)?)?;
            Ok(())
        })?;
    }

    let corefile = render_corefile(paths);
    write_atomic_string_if_changed(&paths.coredns_corefile, &corefile, 0o644, |path| {
        let contents = fs::read_to_string(path)?;
        if !contents.contains("hosts") || !contents.contains(&HUB_VPN_IP.to_string()) {
            bail!("CoreDNS Corefile is missing the minion hosts configuration");
        }
        Ok(())
    })?;

    Ok(())
}

fn render_corefile(paths: &HubPaths) -> String {
    format!(
        ".:53 {{\n    bind {}\n    hosts {} {{\n        ttl 30\n        reload 5s\n        fallthrough\n    }}\n    forward . /etc/resolv.conf\n    errors\n}}\n",
        HUB_VPN_IP,
        paths.coredns_hosts.display()
    )
}

fn ensure_coredns_service(paths: &HubPaths, runner: &dyn CommandRunner) -> Result<()> {
    if runner.enabled() && systemd_unit_exists(runner, "coredns.service") {
        return Ok(());
    }

    let coredns_bin = coredns_service_binary(paths);
    let service = format!(
        "[Unit]\nDescription=CoreDNS DNS server\nAfter=network-online.target wg-quick@minion0.service\nWants=network-online.target\nRequires=wg-quick@minion0.service\n\n[Service]\nType=simple\nExecStart={} -conf {}\nRestart=on-failure\nRestartSec=2\n\n[Install]\nWantedBy=multi-user.target\n",
        coredns_bin.display(),
        paths.coredns_corefile.display()
    );
    write_atomic_string_if_changed(&paths.coredns_service, &service, 0o644, |_| Ok(()))?;
    Ok(())
}

fn coredns_service_binary(paths: &HubPaths) -> PathBuf {
    if paths.coredns_bin.exists() {
        paths.coredns_bin.clone()
    } else {
        PathBuf::from("/usr/bin/coredns")
    }
}

fn systemd_unit_exists(runner: &dyn CommandRunner, unit: &str) -> bool {
    runner
        .run("systemctl", &["list-unit-files", unit])
        .map(|output| output.lines().any(|line| line.starts_with(unit)))
        .unwrap_or(false)
}

fn install_self_binary(paths: &HubPaths) -> Result<()> {
    let current = env::current_exe().context("failed to locate current minion-hub executable")?;
    if same_file_contents(&current, &paths.install_bin)? {
        return Ok(());
    }

    if let Some(parent) = paths.install_bin.parent() {
        fs::create_dir_all(parent)?;
    }

    let tmp = temp_path_for(&paths.install_bin);
    if tmp.exists() {
        let _ = fs::remove_file(&tmp);
    }
    fs::copy(&current, &tmp).with_context(|| {
        format!(
            "failed to copy {} to {}",
            current.display(),
            paths.install_bin.display()
        )
    })?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755))?;
    fs::rename(&tmp, &paths.install_bin)?;
    Ok(())
}

fn same_file_contents(left: &Path, right: &Path) -> Result<bool> {
    if !right.exists() {
        return Ok(false);
    }
    Ok(fs::read(left)? == fs::read(right)?)
}

fn ensure_hub_service(paths: &HubPaths) -> Result<()> {
    let service = format!(
        "[Unit]\nDescription=Minion private hub API\nAfter=network-online.target wg-quick@minion0.service\nWants=network-online.target wg-quick@minion0.service\n\n[Service]\nType=simple\nExecStart={} serve\nRestart=on-failure\nRestartSec=2\n\n[Install]\nWantedBy=multi-user.target\n",
        paths.install_bin.display()
    );
    write_atomic_string_if_changed(&paths.hub_service, &service, 0o644, |_| Ok(()))?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WgConfig {
    private_key: String,
    peers: Vec<WgPeer>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WgPeer {
    vpn_ip: Ipv4Addr,
    public_key: String,
}

fn parse_wireguard_config(contents: &str) -> Result<WgConfig> {
    let mut section = "";
    let mut private_key = None;
    let mut peers = Vec::new();
    let mut current_public_key = None;
    let mut current_vpn_ip = None;

    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            if section == "Peer" {
                peers.push(finish_peer(
                    current_public_key.take(),
                    current_vpn_ip.take(),
                )?);
            }
            section = &line[1..line.len() - 1];
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            bail!("invalid WireGuard config line: {}", raw_line);
        };
        let key = key.trim();
        let value = value.trim();

        match (section, key) {
            ("Interface", "PrivateKey") => private_key = Some(value.to_string()),
            ("Peer", "PublicKey") => {
                validate_wireguard_public_key(value)?;
                current_public_key = Some(value.to_string());
            }
            ("Peer", "AllowedIPs") => {
                current_vpn_ip = Some(parse_peer_allowed_ip(value)?);
            }
            _ => {}
        }
    }

    if section == "Peer" {
        peers.push(finish_peer(current_public_key, current_vpn_ip)?);
    }

    let private_key = private_key.context("WireGuard config is missing Interface PrivateKey")?;
    Ok(WgConfig { private_key, peers })
}

fn validate_wireguard_interface(contents: &str) -> Result<()> {
    let mut section = "";
    let mut address = None;
    let mut listen_port = None;

    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = &line[1..line.len() - 1];
            continue;
        }
        if section != "Interface" {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "Address" => address = Some(value.trim().to_string()),
            "ListenPort" => listen_port = Some(value.trim().parse::<u16>()?),
            _ => {}
        }
    }

    let address = address.context("existing WireGuard config is missing Interface Address")?;
    let has_hub_address = address
        .split(',')
        .map(str::trim)
        .any(|part| part == format!("{}/24", HUB_VPN_IP));
    if !has_hub_address {
        bail!(
            "existing WireGuard config must use Interface Address = {}/24",
            HUB_VPN_IP
        );
    }

    if listen_port.context("existing WireGuard config is missing Interface ListenPort")?
        != WG_LISTEN_PORT
    {
        bail!(
            "existing WireGuard config must use Interface ListenPort = {}",
            WG_LISTEN_PORT
        );
    }

    Ok(())
}

fn finish_peer(public_key: Option<String>, vpn_ip: Option<Ipv4Addr>) -> Result<WgPeer> {
    Ok(WgPeer {
        public_key: public_key.context("WireGuard peer is missing PublicKey")?,
        vpn_ip: vpn_ip.context("WireGuard peer is missing AllowedIPs")?,
    })
}

fn parse_peer_allowed_ip(value: &str) -> Result<Ipv4Addr> {
    for part in value.split(',').map(str::trim) {
        let Some((ip, cidr)) = part.split_once('/') else {
            continue;
        };
        if cidr != "32" {
            continue;
        }
        let ip = ip.parse::<Ipv4Addr>()?;
        if validate_vpn_ip(ip).is_ok() {
            return Ok(ip);
        }
    }
    bail!(
        "WireGuard peer AllowedIPs must include a {} peer /32",
        VPN_CIDR
    )
}

fn render_wireguard_config(config: &WgConfig) -> String {
    let mut peers = config.peers.clone();
    peers.sort_by_key(|peer| peer.vpn_ip.octets());

    let mut rendered = format!(
        "[Interface]\nAddress = {}/24\nListenPort = {}\nPrivateKey = {}\n\n",
        HUB_VPN_IP, WG_LISTEN_PORT, config.private_key
    );

    for peer in peers {
        rendered.push_str("[Peer]\n");
        rendered.push_str(&format!("PublicKey = {}\n", peer.public_key));
        rendered.push_str(&format!("AllowedIPs = {}/32\n\n", peer.vpn_ip));
    }

    rendered
}

fn save_wireguard_config(paths: &HubPaths, config: &WgConfig) -> Result<()> {
    validate_wireguard_state(config)?;
    let rendered = render_wireguard_config(config);
    write_atomic_string(&paths.wg_config, &rendered, 0o600, |path| {
        parse_wireguard_config(&fs::read_to_string(path)?)?;
        Ok(())
    })
}

fn validate_wireguard_state(config: &WgConfig) -> Result<()> {
    let mut ips = BTreeMap::new();
    for peer in &config.peers {
        validate_vpn_ip(peer.vpn_ip)?;
        validate_wireguard_public_key(&peer.public_key)?;
        if ips.insert(peer.vpn_ip, ()).is_some() {
            bail!("VPN IP {} is already assigned", peer.vpn_ip);
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HostRecord {
    name: String,
    vpn_ip: Ipv4Addr,
}

fn parse_hosts(contents: &str) -> Result<Vec<HostRecord>> {
    let mut records = Vec::new();

    for raw_line in contents.lines() {
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }

        let mut parts = line.split_whitespace();
        let ip = parts
            .next()
            .context("CoreDNS hosts line is missing an IP address")?
            .parse::<Ipv4Addr>()?;
        validate_vpn_ip(ip)?;

        let mut saw_name = false;
        for name in parts {
            validate_machine_name(name)?;
            records.push(HostRecord {
                name: name.to_string(),
                vpn_ip: ip,
            });
            saw_name = true;
        }
        if !saw_name {
            bail!("CoreDNS hosts line for {} has no names", ip);
        }
    }

    Ok(records)
}

fn render_hosts(records: &[HostRecord]) -> String {
    let mut records = records.to_vec();
    records.sort_by_key(|record| (record.vpn_ip.octets(), record.name.clone()));

    records
        .iter()
        .map(|record| format!("{} {}\n", record.vpn_ip, record.name))
        .collect()
}

fn save_hosts(paths: &HubPaths, records: &[HostRecord]) -> Result<()> {
    validate_hosts_state(records)?;
    let rendered = render_hosts(records);
    write_atomic_string(&paths.coredns_hosts, &rendered, 0o644, |path| {
        parse_hosts(&fs::read_to_string(path)?)?;
        Ok(())
    })
}

fn validate_hosts_state(records: &[HostRecord]) -> Result<()> {
    let mut names = BTreeMap::new();
    for record in records {
        validate_machine_name(&record.name)?;
        validate_vpn_ip(record.vpn_ip)?;
        if names.insert(record.name.clone(), ()).is_some() {
            bail!("machine name {} already exists", record.name);
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Machine {
    name: String,
    vpn_ip: Ipv4Addr,
    public_key: String,
}

#[derive(Default)]
struct MachinePatch {
    name: Option<String>,
    vpn_ip: Option<Ipv4Addr>,
    public_key: Option<String>,
}

struct HubStore {
    paths: HubPaths,
}

impl HubStore {
    fn new(paths: HubPaths) -> Self {
        Self { paths }
    }

    fn list_machines(&self) -> Result<Vec<Machine>> {
        let wg = self.load_wireguard()?;
        let hosts = self.load_hosts()?;
        let peers_by_ip: HashMap<Ipv4Addr, String> = wg
            .peers
            .into_iter()
            .map(|peer| (peer.vpn_ip, peer.public_key))
            .collect();

        let mut machines = hosts
            .into_iter()
            .filter_map(|record| {
                peers_by_ip.get(&record.vpn_ip).map(|public_key| Machine {
                    name: record.name,
                    vpn_ip: record.vpn_ip,
                    public_key: public_key.clone(),
                })
            })
            .collect::<Vec<_>>();
        machines.sort_by_key(|machine| (machine.vpn_ip.octets(), machine.name.clone()));
        Ok(machines)
    }

    fn get_machine(&self, name: &str) -> Result<Machine> {
        validate_machine_name(name)?;
        self.list_machines()?
            .into_iter()
            .find(|machine| machine.name == name)
            .ok_or_else(|| anyhow!("machine {} not found", name))
    }

    fn add_machine(&self, machine: Machine) -> Result<Machine> {
        validate_machine(&machine)?;
        let mut wg = self.load_wireguard()?;
        let mut hosts = self.load_hosts()?;

        if hosts.iter().any(|record| record.name == machine.name) {
            bail!("machine name {} already exists", machine.name);
        }
        if wg.peers.iter().any(|peer| peer.vpn_ip == machine.vpn_ip)
            || hosts.iter().any(|record| record.vpn_ip == machine.vpn_ip)
        {
            bail!("VPN IP {} is already assigned", machine.vpn_ip);
        }

        wg.peers.push(WgPeer {
            vpn_ip: machine.vpn_ip,
            public_key: machine.public_key.clone(),
        });
        hosts.push(HostRecord {
            name: machine.name.clone(),
            vpn_ip: machine.vpn_ip,
        });

        self.save_state(&wg, &hosts)?;
        Ok(machine)
    }

    fn patch_machine(&self, name: &str, patch: MachinePatch) -> Result<Machine> {
        validate_machine_name(name)?;
        let mut wg = self.load_wireguard()?;
        let mut hosts = self.load_hosts()?;

        let host_index = hosts
            .iter()
            .position(|record| record.name == name)
            .ok_or_else(|| anyhow!("machine {} not found", name))?;
        let old_ip = hosts[host_index].vpn_ip;
        let peer_index = wg
            .peers
            .iter()
            .position(|peer| peer.vpn_ip == old_ip)
            .ok_or_else(|| anyhow!("machine {} has no WireGuard peer", name))?;

        let new_name = patch.name.unwrap_or_else(|| hosts[host_index].name.clone());
        let new_ip = patch.vpn_ip.unwrap_or(old_ip);
        let new_key = patch
            .public_key
            .unwrap_or_else(|| wg.peers[peer_index].public_key.clone());

        validate_machine(&Machine {
            name: new_name.clone(),
            vpn_ip: new_ip,
            public_key: new_key.clone(),
        })?;

        if new_name != name && hosts.iter().any(|record| record.name == new_name) {
            bail!("machine name {} already exists", new_name);
        }
        if new_ip != old_ip
            && (wg.peers.iter().any(|peer| peer.vpn_ip == new_ip)
                || hosts.iter().any(|record| record.vpn_ip == new_ip))
        {
            bail!("VPN IP {} is already assigned", new_ip);
        }

        hosts[host_index].name = new_name.clone();
        hosts[host_index].vpn_ip = new_ip;
        wg.peers[peer_index].vpn_ip = new_ip;
        wg.peers[peer_index].public_key = new_key.clone();

        self.save_state(&wg, &hosts)?;

        Ok(Machine {
            name: new_name,
            vpn_ip: new_ip,
            public_key: new_key,
        })
    }

    fn delete_machine(&self, name: &str) -> Result<()> {
        validate_machine_name(name)?;
        let mut wg = self.load_wireguard()?;
        let mut hosts = self.load_hosts()?;

        let host_index = hosts
            .iter()
            .position(|record| record.name == name)
            .ok_or_else(|| anyhow!("machine {} not found", name))?;
        let vpn_ip = hosts[host_index].vpn_ip;
        hosts.remove(host_index);
        wg.peers.retain(|peer| peer.vpn_ip != vpn_ip);

        self.save_state(&wg, &hosts)?;
        Ok(())
    }

    fn load_wireguard(&self) -> Result<WgConfig> {
        parse_wireguard_config(
            &fs::read_to_string(&self.paths.wg_config)
                .with_context(|| format!("failed to read {}", self.paths.wg_config.display()))?,
        )
    }

    fn load_hosts(&self) -> Result<Vec<HostRecord>> {
        if !self.paths.coredns_hosts.exists() {
            return Ok(Vec::new());
        }
        parse_hosts(
            &fs::read_to_string(&self.paths.coredns_hosts).with_context(|| {
                format!("failed to read {}", self.paths.coredns_hosts.display())
            })?,
        )
    }

    fn save_state(&self, wg: &WgConfig, hosts: &[HostRecord]) -> Result<()> {
        save_wireguard_config(&self.paths, wg)?;
        save_hosts(&self.paths, hosts)?;
        Ok(())
    }
}

fn validate_machine(machine: &Machine) -> Result<()> {
    validate_machine_name(&machine.name)?;
    validate_vpn_ip(machine.vpn_ip)?;
    validate_wireguard_public_key(&machine.public_key)?;
    Ok(())
}

fn validate_machine_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("machine name cannot be empty");
    }
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        bail!(
            "machine name may only contain ASCII letters, numbers, dashes, underscores, and dots"
        );
    }
    Ok(())
}

fn validate_vpn_ip(ip: Ipv4Addr) -> Result<()> {
    let octets = ip.octets();
    if octets[0] != 10 || octets[1] != 42 || octets[2] != 42 {
        bail!("VPN IP {} is outside {}", ip, VPN_CIDR);
    }
    if matches!(octets[3], 0 | 1 | 255) {
        bail!("VPN IP {} is reserved", ip);
    }
    Ok(())
}

fn validate_wireguard_public_key(key: &str) -> Result<()> {
    let decoded = decode_base64(key).with_context(|| "WireGuard public key is not valid base64")?;
    if decoded.len() != 32 {
        bail!("WireGuard public key must decode to 32 bytes");
    }
    if decoded.iter().all(|byte| *byte == 0) {
        bail!("WireGuard public key cannot be all zero bytes");
    }
    Ok(())
}

fn decode_base64(input: &str) -> Result<Vec<u8>> {
    let bytes = input.as_bytes();
    if input.is_empty() || !bytes.chunks_exact(4).remainder().is_empty() {
        bail!("invalid base64 length");
    }

    let mut output = Vec::new();
    let mut saw_padding = false;

    for chunk in bytes.chunks(4) {
        let mut values = [0u8; 4];
        let mut padding = 0;

        for (idx, byte) in chunk.iter().enumerate() {
            match *byte {
                b'A'..=b'Z' if !saw_padding => values[idx] = byte - b'A',
                b'a'..=b'z' if !saw_padding => values[idx] = byte - b'a' + 26,
                b'0'..=b'9' if !saw_padding => values[idx] = byte - b'0' + 52,
                b'+' if !saw_padding => values[idx] = 62,
                b'/' if !saw_padding => values[idx] = 63,
                b'=' => {
                    saw_padding = true;
                    padding += 1;
                    values[idx] = 0;
                }
                _ => bail!("invalid base64 character"),
            }
        }

        output.push((values[0] << 2) | (values[1] >> 4));
        if padding < 2 {
            output.push((values[1] << 4) | (values[2] >> 2));
        }
        if padding == 0 {
            output.push((values[2] << 6) | values[3]);
        }
    }

    Ok(output)
}

fn serve(paths: HubPaths, listen: SocketAddr) -> Result<()> {
    if listen.ip().is_unspecified() {
        bail!("minion-hub serve must not listen on an unspecified address");
    }
    if listen.ip() != HUB_VPN_IP {
        bail!(
            "minion-hub serve must listen on private WireGuard address {}",
            HUB_VPN_IP
        );
    }

    let listener =
        TcpListener::bind(listen).with_context(|| format!("failed to bind {}", listen))?;
    let runner = SystemCommandRunner::new(false);

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let response = match read_http_request(&mut stream) {
                    Ok(request) => route_request(&paths, &runner, request),
                    Err(error) => HttpResponse::json_error(400, &error.to_string()),
                };
                let _ = stream.write_all(&response.to_bytes());
            }
            Err(error) => eprintln!("failed to accept connection: {}", error),
        }
    }

    Ok(())
}

#[derive(Debug)]
struct HttpRequest {
    method: String,
    path: String,
    body: String,
}

#[derive(Debug)]
struct HttpResponse {
    status: u16,
    body: String,
}

impl HttpResponse {
    fn json(status: u16, body: String) -> Self {
        Self { status, body }
    }

    fn json_error(status: u16, message: &str) -> Self {
        Self {
            status,
            body: format!("{{\"error\":\"{}\"}}", json_escape(message)),
        }
    }

    fn to_bytes(&self) -> Vec<u8> {
        let reason = match self.status {
            200 => "OK",
            201 => "Created",
            204 => "No Content",
            400 => "Bad Request",
            404 => "Not Found",
            405 => "Method Not Allowed",
            409 => "Conflict",
            _ => "Internal Server Error",
        };
        let body = if self.status == 204 {
            String::new()
        } else {
            self.body.clone()
        };
        format!(
            "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            self.status,
            reason,
            body.len(),
            body
        )
        .into_bytes()
    }
}

fn read_http_request(stream: &mut TcpStream) -> Result<HttpRequest> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];

    loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if buffer.len() > 16 * 1024 {
            bail!("HTTP headers are too large");
        }
    }

    let header_end = buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|idx| idx + 4)
        .context("HTTP request is missing header terminator")?;

    let header = String::from_utf8(buffer[..header_end].to_vec())?;
    let mut lines = header.split("\r\n");
    let request_line = lines
        .next()
        .context("HTTP request is missing request line")?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .context("HTTP request is missing method")?
        .to_string();
    let path = request_parts
        .next()
        .context("HTTP request is missing path")?
        .to_string();

    let content_length = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.trim().parse::<usize>())
        .transpose()?
        .unwrap_or(0);
    if content_length > 64 * 1024 {
        bail!("HTTP body is too large");
    }

    let mut body = buffer[header_end..].to_vec();
    while body.len() < content_length {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(content_length);

    Ok(HttpRequest {
        method,
        path,
        body: String::from_utf8(body)?,
    })
}

fn route_request(
    paths: &HubPaths,
    runner: &dyn CommandRunner,
    request: HttpRequest,
) -> HttpResponse {
    let path = request.path.split('?').next().unwrap_or(&request.path);
    let store = HubStore::new(paths.clone());

    let result = match (request.method.as_str(), path) {
        ("GET", "/machines") => store
            .list_machines()
            .map(|machines| HttpResponse::json(200, machines_json(&machines))),
        ("POST", "/machines") => parse_machine_body(&request.body)
            .and_then(|machine| store.add_machine(machine))
            .and_then(|machine| {
                apply_runtime_changes(paths, runner)?;
                Ok(HttpResponse::json(201, machine_json(&machine)))
            }),
        _ if path.starts_with("/machines/") => {
            let name = &path["/machines/".len()..];
            route_machine_request(&store, paths, runner, &request, name)
        }
        _ => Ok(HttpResponse::json_error(404, "not found")),
    };

    result.unwrap_or_else(error_response)
}

fn route_machine_request(
    store: &HubStore,
    paths: &HubPaths,
    runner: &dyn CommandRunner,
    request: &HttpRequest,
    name: &str,
) -> Result<HttpResponse> {
    match request.method.as_str() {
        "GET" => store
            .get_machine(name)
            .map(|machine| HttpResponse::json(200, machine_json(&machine))),
        "PATCH" => parse_machine_patch_body(&request.body)
            .and_then(|patch| store.patch_machine(name, patch))
            .and_then(|machine| {
                apply_runtime_changes(paths, runner)?;
                Ok(HttpResponse::json(200, machine_json(&machine)))
            }),
        "DELETE" => store.delete_machine(name).and_then(|_| {
            apply_runtime_changes(paths, runner)?;
            Ok(HttpResponse::json(204, String::new()))
        }),
        _ => Ok(HttpResponse::json_error(405, "method not allowed")),
    }
}

fn error_response(error: anyhow::Error) -> HttpResponse {
    let message = error.to_string();
    if message.contains("not found") {
        HttpResponse::json_error(404, &message)
    } else if message.contains("already exists") || message.contains("already assigned") {
        HttpResponse::json_error(409, &message)
    } else {
        HttpResponse::json_error(400, &message)
    }
}

fn apply_runtime_changes(paths: &HubPaths, runner: &dyn CommandRunner) -> Result<()> {
    apply_wireguard_config(paths, runner)?;
    reload_coredns(runner)?;
    Ok(())
}

fn apply_wireguard_config(paths: &HubPaths, runner: &dyn CommandRunner) -> Result<()> {
    if !runner.enabled() {
        return Ok(());
    }

    let config_path = paths.wg_config.to_string_lossy().to_string();
    let stripped = match runner.run("wg-quick", &["strip", &config_path]) {
        Ok(stripped) => stripped,
        Err(_) => {
            runner.run("systemctl", &["restart", "wg-quick@minion0"])?;
            return Ok(());
        }
    };

    let sync_path = env::temp_dir().join(format!("minion0.syncconf.{}", std::process::id()));
    fs::write(&sync_path, stripped)?;
    let sync_path_string = sync_path.to_string_lossy().to_string();
    let result = runner.run("wg", &["syncconf", INTERFACE, &sync_path_string]);
    let _ = fs::remove_file(&sync_path);

    if result.is_err() {
        runner.run("systemctl", &["restart", "wg-quick@minion0"])?;
    } else {
        result?;
    }

    Ok(())
}

fn reload_coredns(runner: &dyn CommandRunner) -> Result<()> {
    if !runner.enabled() {
        return Ok(());
    }

    if runner.run("systemctl", &["reload", "coredns"]).is_err() {
        runner.run("systemctl", &["restart", "coredns"])?;
    }
    Ok(())
}

fn parse_machine_body(body: &str) -> Result<Machine> {
    let object = parse_json_object(body)?;
    let name = required_json_string(&object, "name")?;
    let vpn_ip = required_json_string(&object, "vpn_ip")?.parse::<Ipv4Addr>()?;
    let public_key = required_json_string(&object, "public_key")?;
    Ok(Machine {
        name,
        vpn_ip,
        public_key,
    })
}

fn parse_machine_patch_body(body: &str) -> Result<MachinePatch> {
    let object = parse_json_object(body)?;
    let vpn_ip = object
        .get("vpn_ip")
        .map(|value| value.parse::<Ipv4Addr>())
        .transpose()?;
    Ok(MachinePatch {
        name: object.get("name").cloned(),
        vpn_ip,
        public_key: object.get("public_key").cloned(),
    })
}

fn required_json_string(object: &BTreeMap<String, String>, key: &str) -> Result<String> {
    object
        .get(key)
        .cloned()
        .ok_or_else(|| anyhow!("request body is missing {}", key))
}

fn machines_json(machines: &[Machine]) -> String {
    format!(
        "[{}]",
        machines
            .iter()
            .map(machine_json)
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn machine_json(machine: &Machine) -> String {
    format!(
        "{{\"name\":\"{}\",\"vpn_ip\":\"{}\",\"public_key\":\"{}\"}}",
        json_escape(&machine.name),
        machine.vpn_ip,
        json_escape(&machine.public_key)
    )
}

fn parse_json_object(input: &str) -> Result<BTreeMap<String, String>> {
    let bytes = input.as_bytes();
    let mut idx = 0;
    skip_json_ws(bytes, &mut idx);
    expect_json_byte(bytes, &mut idx, b'{')?;
    skip_json_ws(bytes, &mut idx);

    let mut object = BTreeMap::new();
    if peek_json_byte(bytes, idx) == Some(b'}') {
        idx += 1;
        skip_json_ws(bytes, &mut idx);
        if idx != bytes.len() {
            bail!("unexpected data after JSON object");
        }
        return Ok(object);
    }

    loop {
        let key = parse_json_string(bytes, &mut idx)?;
        skip_json_ws(bytes, &mut idx);
        expect_json_byte(bytes, &mut idx, b':')?;
        skip_json_ws(bytes, &mut idx);
        let value = parse_json_string(bytes, &mut idx)?;
        object.insert(key, value);
        skip_json_ws(bytes, &mut idx);

        match peek_json_byte(bytes, idx) {
            Some(b',') => {
                idx += 1;
                skip_json_ws(bytes, &mut idx);
            }
            Some(b'}') => {
                idx += 1;
                break;
            }
            _ => bail!("expected comma or end of JSON object"),
        }
    }

    skip_json_ws(bytes, &mut idx);
    if idx != bytes.len() {
        bail!("unexpected data after JSON object");
    }
    Ok(object)
}

fn parse_json_string(bytes: &[u8], idx: &mut usize) -> Result<String> {
    expect_json_byte(bytes, idx, b'"')?;
    let mut value = String::new();

    while let Some(byte) = peek_json_byte(bytes, *idx) {
        *idx += 1;
        match byte {
            b'"' => return Ok(value),
            b'\\' => {
                let escaped = peek_json_byte(bytes, *idx).context("unterminated JSON escape")?;
                *idx += 1;
                match escaped {
                    b'"' => value.push('"'),
                    b'\\' => value.push('\\'),
                    b'/' => value.push('/'),
                    b'b' => value.push('\u{0008}'),
                    b'f' => value.push('\u{000c}'),
                    b'n' => value.push('\n'),
                    b'r' => value.push('\r'),
                    b't' => value.push('\t'),
                    _ => bail!("unsupported JSON escape"),
                }
            }
            0..=31 => bail!("control character in JSON string"),
            32..=126 => value.push(byte as char),
            _ => bail!("only ASCII JSON strings are supported"),
        }
    }

    bail!("unterminated JSON string")
}

fn skip_json_ws(bytes: &[u8], idx: &mut usize) {
    while matches!(
        peek_json_byte(bytes, *idx),
        Some(b' ' | b'\n' | b'\r' | b'\t')
    ) {
        *idx += 1;
    }
}

fn expect_json_byte(bytes: &[u8], idx: &mut usize, expected: u8) -> Result<()> {
    match peek_json_byte(bytes, *idx) {
        Some(actual) if actual == expected => {
            *idx += 1;
            Ok(())
        }
        _ => bail!("expected JSON byte {}", expected as char),
    }
}

fn peek_json_byte(bytes: &[u8], idx: usize) -> Option<u8> {
    bytes.get(idx).copied()
}

fn json_escape(input: &str) -> String {
    let mut escaped = String::new();
    for ch in input.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn write_atomic_string<F>(path: &Path, contents: &str, mode: u32, validate: F) -> Result<()>
where
    F: FnOnce(&Path) -> Result<()>,
{
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let tmp = temp_path_for(path);
    if tmp.exists() {
        let _ = fs::remove_file(&tmp);
    }

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .with_context(|| format!("failed to create {}", tmp.display()))?;
    file.set_permissions(fs::Permissions::from_mode(mode))?;
    file.write_all(contents.as_bytes())?;
    file.sync_all()?;
    drop(file);

    validate(&tmp)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn write_atomic_string_if_changed<F>(
    path: &Path,
    contents: &str,
    mode: u32,
    validate: F,
) -> Result<bool>
where
    F: FnOnce(&Path) -> Result<()>,
{
    if path.exists() && fs::read_to_string(path)? == contents {
        return Ok(false);
    }
    write_atomic_string(path, contents, mode, validate)?;
    Ok(true)
}

fn temp_path_for(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("minion-hub");
    path.with_file_name(format!(".{}.tmp.{}", file_name, std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use tempfile::tempdir;

    const PEER_KEY: &str = "e5ryaK4/d3GriUqOgaGnNRWYglvSXXmLwSmSsTvPZDk=";
    const PEER_KEY_2: &str = "n3ST37Z5J9dm00wbUPn2mfNOGIo3iHnMKmtdSJuJONE=";

    #[derive(Default)]
    struct NoopRunner {
        commands: RefCell<Vec<String>>,
    }

    impl CommandRunner for NoopRunner {
        fn enabled(&self) -> bool {
            false
        }

        fn run(&self, program: &str, args: &[&str]) -> Result<String> {
            self.commands
                .borrow_mut()
                .push(format!("{} {}", program, args.join(" ")));
            Ok(String::new())
        }

        fn run_with_stdin(&self, program: &str, args: &[&str], _stdin: &str) -> Result<String> {
            self.run(program, args)
        }
    }

    struct ScriptedRunner {
        responses: RefCell<VecDeque<Result<String, String>>>,
        commands: RefCell<Vec<Vec<String>>>,
    }

    impl ScriptedRunner {
        fn new(responses: Vec<Result<&str, &str>>) -> Self {
            Self {
                responses: RefCell::new(
                    responses
                        .into_iter()
                        .map(|response| {
                            response
                                .map(|value| value.to_string())
                                .map_err(|value| value.to_string())
                        })
                        .collect(),
                ),
                commands: RefCell::new(Vec::new()),
            }
        }

        fn commands(&self) -> Vec<Vec<String>> {
            self.commands.borrow().clone()
        }
    }

    impl CommandRunner for ScriptedRunner {
        fn enabled(&self) -> bool {
            true
        }

        fn run(&self, program: &str, args: &[&str]) -> Result<String> {
            let mut command = vec![program.to_string()];
            command.extend(args.iter().map(|arg| arg.to_string()));
            self.commands.borrow_mut().push(command);

            match self
                .responses
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(|| Ok(String::new()))
            {
                Ok(output) => Ok(output),
                Err(error) => Err(anyhow!(error)),
            }
        }

        fn run_with_stdin(&self, program: &str, args: &[&str], _stdin: &str) -> Result<String> {
            self.run(program, args)
        }
    }

    fn prepared_store() -> (tempfile::TempDir, HubPaths, NoopRunner) {
        let dir = tempdir().unwrap();
        let paths = HubPaths::under_root(dir.path());
        let config = WgConfig {
            private_key: TEST_PRIVATE_KEY.to_string(),
            peers: Vec::new(),
        };
        save_wireguard_config(&paths, &config).unwrap();
        save_hosts(&paths, &[]).unwrap();
        (dir, paths, NoopRunner::default())
    }

    fn request(method: &str, path: &str, body: &str) -> HttpRequest {
        HttpRequest {
            method: method.to_string(),
            path: path.to_string(),
            body: body.to_string(),
        }
    }

    #[test]
    fn validates_machine_names() {
        assert!(validate_machine_name("web-01.prod").is_ok());
        assert!(validate_machine_name("web_01").is_ok());
        assert!(validate_machine_name("bad name").is_err());
        assert!(validate_machine_name("bad;name").is_err());
    }

    #[test]
    fn validates_private_vpn_ips() {
        assert!(validate_vpn_ip(Ipv4Addr::new(10, 42, 42, 2)).is_ok());
        assert!(validate_vpn_ip(Ipv4Addr::new(10, 42, 42, 1)).is_err());
        assert!(validate_vpn_ip(Ipv4Addr::new(10, 42, 42, 255)).is_err());
        assert!(validate_vpn_ip(Ipv4Addr::new(192, 168, 1, 2)).is_err());
    }

    #[test]
    fn validates_wireguard_public_keys() {
        assert!(validate_wireguard_public_key(PEER_KEY).is_ok());
        assert!(validate_wireguard_public_key("not-a-key").is_err());
        assert!(
            validate_wireguard_public_key("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").is_err()
        );
    }

    #[test]
    fn install_packages_downloads_coredns_when_apt_package_is_missing() {
        let runner = ScriptedRunner::new(vec![
            Ok(""),
            Ok(""),
            Err("E: No packages found"),
            Err("missing coredns"),
            Ok("amd64\n"),
            Ok(""),
            Ok(""),
            Ok(""),
            Ok(""),
        ]);

        install_packages(&runner).unwrap();

        let commands = runner.commands();
        assert_eq!(commands[0], vec!["apt-get", "update"]);
        assert_eq!(
            commands[1],
            vec![
                "apt-get",
                "install",
                "-y",
                "wireguard",
                "wireguard-tools",
                "ca-certificates",
                "curl"
            ]
        );
        assert_eq!(commands[2], vec!["apt-cache", "show", "coredns"]);
        assert_eq!(commands[3], vec!["test", "-x", "/usr/local/bin/coredns"]);
        assert_eq!(commands[4], vec!["dpkg", "--print-architecture"]);
        assert_eq!(commands[5][0], "curl");
        assert!(commands[5].iter().any(|arg| arg.contains("linux_amd64")));
        assert_eq!(commands[6][0], "tar");
        assert_eq!(commands[7][0], "install");
    }

    #[test]
    fn coredns_service_uses_fallback_binary_when_present() {
        let dir = tempdir().unwrap();
        let paths = HubPaths::under_root(dir.path());
        write_atomic_string(&paths.coredns_bin, "", 0o755, |_| Ok(())).unwrap();

        ensure_coredns_service(&paths, &NoopRunner::default()).unwrap();

        let service = fs::read_to_string(&paths.coredns_service).unwrap();
        assert!(service.contains(&format!("ExecStart={} -conf", paths.coredns_bin.display())));
        assert!(service.contains("After=network-online.target wg-quick@minion0.service"));
        assert!(service.contains("Requires=wg-quick@minion0.service"));
    }

    #[test]
    fn hub_service_waits_for_wireguard_without_hard_requirement() {
        let dir = tempdir().unwrap();
        let paths = HubPaths::under_root(dir.path());

        ensure_hub_service(&paths).unwrap();

        let service = fs::read_to_string(&paths.hub_service).unwrap();
        assert!(service.contains("After=network-online.target wg-quick@minion0.service"));
        assert!(service.contains("Wants=network-online.target wg-quick@minion0.service"));
        assert!(!service.contains("Requires=wg-quick@minion0.service"));
    }

    #[test]
    fn corefile_forwards_non_minion_dns() {
        let dir = tempdir().unwrap();
        let paths = HubPaths::under_root(dir.path());

        let corefile = render_corefile(&paths);

        assert!(corefile.contains("hosts "));
        assert!(corefile.contains("forward . /etc/resolv.conf"));
    }

    #[test]
    fn validates_existing_wireguard_interface_settings() {
        let valid = format!(
            "[Interface]\nAddress = {}/24\nListenPort = {}\nPrivateKey = {}\n",
            HUB_VPN_IP, WG_LISTEN_PORT, TEST_PRIVATE_KEY
        );
        assert!(validate_wireguard_interface(&valid).is_ok());

        let wrong_address = valid.replace("10.42.42.1/24", "10.42.42.9/24");
        assert!(validate_wireguard_interface(&wrong_address).is_err());

        let wrong_port = valid.replace("51820", "51821");
        assert!(validate_wireguard_interface(&wrong_port).is_err());
    }

    #[test]
    fn parses_and_renders_wireguard_config() {
        let rendered = render_wireguard_config(&WgConfig {
            private_key: TEST_PRIVATE_KEY.to_string(),
            peers: vec![WgPeer {
                vpn_ip: Ipv4Addr::new(10, 42, 42, 2),
                public_key: PEER_KEY.to_string(),
            }],
        });

        assert!(!rendered.contains("SaveConfig"));
        let parsed = parse_wireguard_config(&rendered).unwrap();
        assert_eq!(parsed.private_key, TEST_PRIVATE_KEY);
        assert_eq!(parsed.peers.len(), 1);
        assert_eq!(parsed.peers[0].vpn_ip, Ipv4Addr::new(10, 42, 42, 2));
    }

    #[test]
    fn init_file_writes_are_idempotent_for_existing_wireguard_config() {
        let dir = tempdir().unwrap();
        let paths = HubPaths::under_root(dir.path());
        ensure_wireguard_config(&paths, TEST_PRIVATE_KEY).unwrap();
        let first = fs::read_to_string(&paths.wg_config).unwrap();

        ensure_wireguard_config(&paths, "different-private-key").unwrap();
        let second = fs::read_to_string(&paths.wg_config).unwrap();

        assert_eq!(first, second);
        assert!(second.contains(TEST_PRIVATE_KEY));
        assert!(!second.contains("different-private-key"));
    }

    #[test]
    fn init_preserves_existing_coredns_hosts() {
        let dir = tempdir().unwrap();
        let paths = HubPaths::under_root(dir.path());
        save_hosts(
            &paths,
            &[HostRecord {
                name: "web-01".to_string(),
                vpn_ip: Ipv4Addr::new(10, 42, 42, 2),
            }],
        )
        .unwrap();

        ensure_coredns_files(&paths).unwrap();

        assert_eq!(
            fs::read_to_string(&paths.coredns_hosts).unwrap(),
            "10.42.42.2 web-01\n"
        );
    }

    #[test]
    fn api_crud_updates_wireguard_and_coredns_files() {
        let (_dir, paths, runner) = prepared_store();

        let created = route_request(
            &paths,
            &runner,
            request(
                "POST",
                "/machines",
                &format!(
                    "{{\"name\":\"web-01\",\"vpn_ip\":\"10.42.42.2\",\"public_key\":\"{}\"}}",
                    PEER_KEY
                ),
            ),
        );
        assert_eq!(created.status, 201);
        assert!(fs::read_to_string(&paths.wg_config)
            .unwrap()
            .contains("AllowedIPs = 10.42.42.2/32"));
        assert_eq!(
            fs::read_to_string(&paths.coredns_hosts).unwrap(),
            "10.42.42.2 web-01\n"
        );

        let listed = route_request(&paths, &runner, request("GET", "/machines", ""));
        assert_eq!(listed.status, 200);
        assert!(listed.body.contains("\"name\":\"web-01\""));

        let patched = route_request(
            &paths,
            &runner,
            request(
                "PATCH",
                "/machines/web-01",
                &format!(
                    "{{\"name\":\"web_02\",\"vpn_ip\":\"10.42.42.3\",\"public_key\":\"{}\"}}",
                    PEER_KEY_2
                ),
            ),
        );
        assert_eq!(patched.status, 200);
        assert!(fs::read_to_string(&paths.wg_config)
            .unwrap()
            .contains("AllowedIPs = 10.42.42.3/32"));
        assert_eq!(
            fs::read_to_string(&paths.coredns_hosts).unwrap(),
            "10.42.42.3 web_02\n"
        );

        let deleted = route_request(&paths, &runner, request("DELETE", "/machines/web_02", ""));
        assert_eq!(deleted.status, 204);
        assert!(!fs::read_to_string(&paths.wg_config)
            .unwrap()
            .contains("[Peer]"));
        assert_eq!(fs::read_to_string(&paths.coredns_hosts).unwrap(), "");
    }

    #[test]
    fn api_rejects_invalid_input_and_duplicate_ips() {
        let (_dir, paths, runner) = prepared_store();

        let invalid_name = route_request(
            &paths,
            &runner,
            request(
                "POST",
                "/machines",
                &format!(
                    "{{\"name\":\"bad;name\",\"vpn_ip\":\"10.42.42.2\",\"public_key\":\"{}\"}}",
                    PEER_KEY
                ),
            ),
        );
        assert_eq!(invalid_name.status, 400);

        let invalid_ip = route_request(
            &paths,
            &runner,
            request(
                "POST",
                "/machines",
                &format!(
                    "{{\"name\":\"web\",\"vpn_ip\":\"10.42.42.1\",\"public_key\":\"{}\"}}",
                    PEER_KEY
                ),
            ),
        );
        assert_eq!(invalid_ip.status, 400);

        let invalid_key = route_request(
            &paths,
            &runner,
            request(
                "POST",
                "/machines",
                "{\"name\":\"web\",\"vpn_ip\":\"10.42.42.2\",\"public_key\":\"bad\"}",
            ),
        );
        assert_eq!(invalid_key.status, 400);

        let first = route_request(
            &paths,
            &runner,
            request(
                "POST",
                "/machines",
                &format!(
                    "{{\"name\":\"web\",\"vpn_ip\":\"10.42.42.2\",\"public_key\":\"{}\"}}",
                    PEER_KEY
                ),
            ),
        );
        assert_eq!(first.status, 201);

        let duplicate_ip = route_request(
            &paths,
            &runner,
            request(
                "POST",
                "/machines",
                &format!(
                    "{{\"name\":\"db\",\"vpn_ip\":\"10.42.42.2\",\"public_key\":\"{}\"}}",
                    PEER_KEY_2
                ),
            ),
        );
        assert_eq!(duplicate_ip.status, 409);
    }
}
