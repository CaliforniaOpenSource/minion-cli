//! CRUD store built from WireGuard peers and CoreDNS host records.

use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashMap;
use std::fs;
use std::net::Ipv4Addr;

use crate::model::{
    parse_hosts, parse_wireguard_config, save_hosts, save_wireguard_config, validate_machine_name,
    validate_vpn_ip, validate_wireguard_public_key, HostRecord, WgConfig, WgPeer,
};
use crate::paths::HubPaths;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Machine {
    pub(crate) name: String,
    pub(crate) vpn_ip: Ipv4Addr,
    pub(crate) public_key: String,
}

#[derive(Default)]
pub(crate) struct MachinePatch {
    pub(crate) name: Option<String>,
    pub(crate) vpn_ip: Option<Ipv4Addr>,
    pub(crate) public_key: Option<String>,
}

pub(crate) struct HubStore {
    paths: HubPaths,
}

impl HubStore {
    pub(crate) fn new(paths: HubPaths) -> Self {
        Self { paths }
    }

    // http::error_response maps "not found", "already exists", and
    // "already assigned" substrings to response statuses until store errors are typed.
    pub(crate) fn list_machines(&self) -> Result<Vec<Machine>> {
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

    pub(crate) fn get_machine(&self, name: &str) -> Result<Machine> {
        validate_machine_name(name)?;
        self.list_machines()?
            .into_iter()
            .find(|machine| machine.name == name)
            .ok_or_else(|| anyhow!("machine {} not found", name))
    }

    pub(crate) fn add_machine(&self, machine: Machine) -> Result<Machine> {
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

    pub(crate) fn patch_machine(&self, name: &str, patch: MachinePatch) -> Result<Machine> {
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

    pub(crate) fn delete_machine(&self, name: &str) -> Result<()> {
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
