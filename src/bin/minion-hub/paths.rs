//! Filesystem locations used by the hub.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub(crate) struct HubPaths {
    pub(crate) wg_config: PathBuf,
    pub(crate) wg_public_key: PathBuf,
    pub(crate) coredns_corefile: PathBuf,
    pub(crate) coredns_hosts: PathBuf,
    pub(crate) coredns_bin: PathBuf,
    pub(crate) coredns_service: PathBuf,
    pub(crate) hub_service: PathBuf,
    pub(crate) sysctl_file: PathBuf,
    pub(crate) install_bin: PathBuf,
}

impl HubPaths {
    pub(crate) fn default() -> Self {
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

    pub(crate) fn under_root(root: &Path) -> Self {
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
