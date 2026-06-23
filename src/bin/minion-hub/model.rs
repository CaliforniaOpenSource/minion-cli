//! WireGuard and CoreDNS hosts parsing, rendering, and validation.

use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;
use std::fs;
use std::net::Ipv4Addr;

use crate::base64::decode_base64;
use crate::fs_atomic::write_atomic_string;
use crate::paths::HubPaths;
use crate::{HUB_VPN_IP, VPN_CIDR, WG_LISTEN_PORT};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WgConfig {
    pub(crate) private_key: String,
    pub(crate) peers: Vec<WgPeer>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WgPeer {
    pub(crate) vpn_ip: Ipv4Addr,
    pub(crate) public_key: String,
}

pub(crate) fn parse_wireguard_config(contents: &str) -> Result<WgConfig> {
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

pub(crate) fn validate_wireguard_interface(contents: &str) -> Result<()> {
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

pub(crate) fn render_wireguard_config(config: &WgConfig) -> String {
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

pub(crate) fn save_wireguard_config(paths: &HubPaths, config: &WgConfig) -> Result<()> {
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
pub(crate) struct HostRecord {
    pub(crate) name: String,
    pub(crate) vpn_ip: Ipv4Addr,
}

pub(crate) fn parse_hosts(contents: &str) -> Result<Vec<HostRecord>> {
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

pub(crate) fn save_hosts(paths: &HubPaths, records: &[HostRecord]) -> Result<()> {
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

pub(crate) fn validate_machine_name(name: &str) -> Result<()> {
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

pub(crate) fn validate_vpn_ip(ip: Ipv4Addr) -> Result<()> {
    let octets = ip.octets();
    if octets[0] != 10 || octets[1] != 42 || octets[2] != 42 {
        bail!("VPN IP {} is outside {}", ip, VPN_CIDR);
    }
    if matches!(octets[3], 0 | 1 | 255) {
        bail!("VPN IP {} is reserved", ip);
    }
    Ok(())
}

pub(crate) fn validate_wireguard_public_key(key: &str) -> Result<()> {
    let decoded = decode_base64(key).with_context(|| "WireGuard public key is not valid base64")?;
    if decoded.len() != 32 {
        bail!("WireGuard public key must decode to 32 bytes");
    }
    if decoded.iter().all(|byte| *byte == 0) {
        bail!("WireGuard public key cannot be all zero bytes");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{PEER_KEY, TEST_PRIVATE_KEY};
    use std::net::Ipv4Addr;

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
}
