//! Applying file changes to running WireGuard and CoreDNS services.

use anyhow::Result;
use std::env;
use std::fs;

use crate::command::CommandRunner;
use crate::paths::HubPaths;
use crate::{wg_quick_unit, INTERFACE};

pub(crate) fn apply_runtime_changes(paths: &HubPaths, runner: &dyn CommandRunner) -> Result<()> {
    apply_wireguard_config(paths, runner)?;
    reload_coredns(runner)?;
    Ok(())
}

fn apply_wireguard_config(paths: &HubPaths, runner: &dyn CommandRunner) -> Result<()> {
    if !runner.enabled() {
        return Ok(());
    }

    let config_path = paths.wg_config.to_string_lossy().to_string();
    let wg_unit = wg_quick_unit();
    let stripped = match runner.run("wg-quick", &["strip", &config_path]) {
        Ok(stripped) => stripped,
        Err(_) => {
            runner.run("systemctl", &["restart", &wg_unit])?;
            return Ok(());
        }
    };

    let sync_path = env::temp_dir().join(format!("{}.syncconf.{}", INTERFACE, std::process::id()));
    fs::write(&sync_path, stripped)?;
    let sync_path_string = sync_path.to_string_lossy().to_string();
    let result = runner.run("wg", &["syncconf", INTERFACE, &sync_path_string]);
    let _ = fs::remove_file(&sync_path);

    if result.is_err() {
        runner.run("systemctl", &["restart", &wg_unit])?;
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
