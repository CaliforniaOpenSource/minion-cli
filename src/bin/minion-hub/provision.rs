//! Provisioning for a fresh private hub host.

use anyhow::{bail, Context, Result};
use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::command::CommandRunner;
use crate::fs_atomic::{temp_path_for, write_atomic_string, write_atomic_string_if_changed};
use crate::model::{
    parse_hosts, parse_wireguard_config, save_wireguard_config, validate_wireguard_interface,
    WgConfig,
};
use crate::paths::HubPaths;
use crate::{
    wg_quick_unit, API_LISTEN_ADDR, COREDNS_VERSION, HUB_VPN_IP, TEST_PRIVATE_KEY, TEST_PUBLIC_KEY,
};

pub(crate) fn init_hub(paths: &HubPaths, runner: &dyn CommandRunner) -> Result<()> {
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
        let wg_unit = wg_quick_unit();
        runner.run("systemctl", &["enable", "--now", &wg_unit])?;
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
    let wg_unit = wg_quick_unit();
    let service = format!(
        "[Unit]\nDescription=CoreDNS DNS server\nAfter=network-online.target {}.service\nWants=network-online.target\nRequires={}.service\n\n[Service]\nType=simple\nExecStart={} -conf {}\nRestart=on-failure\nRestartSec=2\n\n[Install]\nWantedBy=multi-user.target\n",
        wg_unit,
        wg_unit,
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
    let wg_unit = wg_quick_unit();
    let service = format!(
        "[Unit]\nDescription=Minion private hub API\nAfter=network-online.target {}.service\nWants=network-online.target {}.service\n\n[Service]\nType=simple\nExecStart={} serve\nRestart=on-failure\nRestartSec=2\n\n[Install]\nWantedBy=multi-user.target\n",
        wg_unit,
        wg_unit,
        paths.install_bin.display()
    );
    write_atomic_string_if_changed(&paths.hub_service, &service, 0o644, |_| Ok(()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs_atomic::write_atomic_string;
    use crate::model::{save_hosts, HostRecord};
    use crate::test_support::{NoopRunner, ScriptedRunner, TEST_PRIVATE_KEY};
    use std::fs;
    use std::net::Ipv4Addr;
    use tempfile::tempdir;

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
}
