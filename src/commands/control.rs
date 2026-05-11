use anyhow::{anyhow, Result};

use crate::utils::{AppConfig, AppConfigOverrides, SshClient};

pub struct ControlCommand;

#[derive(Debug, Clone)]
pub enum ControlAction {
    Status,
    Ps,
    Logs { follow: bool, tail: u16 },
    Restart,
    Stop,
    Start,
    Doctor,
}

impl ControlCommand {
    pub fn new() -> Self {
        ControlCommand
    }

    pub fn execute(&self, action: ControlAction, overrides: AppConfigOverrides) -> Result<()> {
        let config = AppConfig::load(overrides, false, false)?;
        config.require_app_control()?;

        println!("Connecting to {} as {}...", config.host, config.ssh_user);
        let client =
            SshClient::connect_with_auth(&config.host, &config.ssh_user, &config.ssh_auth())?;

        match action {
            ControlAction::Status => self.status(&client, &config),
            ControlAction::Ps => self.ps(&client, &config),
            ControlAction::Logs { follow, tail } => self.logs(&client, &config, follow, tail),
            ControlAction::Restart => self.compose_action(&client, &config, "restart"),
            ControlAction::Stop => self.compose_action(&client, &config, "stop"),
            ControlAction::Start => self.compose_action(&client, &config, "up -d"),
            ControlAction::Doctor => self.doctor(&client, &config),
        }
    }

    fn status(&self, client: &SshClient, config: &AppConfig) -> Result<()> {
        self.ensure_app(client, config)?;
        let command = format!(
            "cd {} && docker compose ps && printf '\\nRecent logs:\\n' && docker compose logs --tail 40 2>&1",
            shell_quote(&app_dir(config))
        );
        run_and_print(client, &command, "Failed to read app status")
    }

    fn ps(&self, client: &SshClient, config: &AppConfig) -> Result<()> {
        self.ensure_app(client, config)?;
        let command = format!("cd {} && docker compose ps", shell_quote(&app_dir(config)));
        run_and_print(client, &command, "Failed to list app containers")
    }

    fn logs(&self, client: &SshClient, config: &AppConfig, follow: bool, tail: u16) -> Result<()> {
        self.ensure_app(client, config)?;

        let follow_arg = if follow { " --follow" } else { "" };
        let command = format!(
            "cd {} && docker compose logs --tail {}{} 2>&1",
            shell_quote(&app_dir(config)),
            tail,
            follow_arg
        );
        let status = client.execute_command_stream(&command)?;
        if status != 0 {
            return Err(anyhow!("Failed to read app logs"));
        }

        Ok(())
    }

    fn compose_action(&self, client: &SshClient, config: &AppConfig, action: &str) -> Result<()> {
        self.ensure_app(client, config)?;
        let command = format!(
            "cd {} && docker compose {}",
            shell_quote(&app_dir(config)),
            action
        );
        run_and_print(client, &command, "Failed to control app")
    }

    fn doctor(&self, client: &SshClient, config: &AppConfig) -> Result<()> {
        let mut healthy = true;

        healthy &= self.check(client, "Remote Docker is installed", "docker --version")?;
        healthy &= self.check(
            client,
            "Remote Docker Compose is installed",
            "docker compose version",
        )?;
        healthy &= self.check(
            client,
            "Traefik container is running",
            "docker ps --filter 'name=traefik' --format '{{.Status}}' | grep -q Up",
        )?;
        healthy &= self.check(
            client,
            "App compose file exists",
            &format!(
                "test -f {}/docker-compose.yml",
                shell_quote(&app_dir(config))
            ),
        )?;

        if healthy {
            println!("[ok] Basic server checks passed");
            self.ps(client, config)?;
            return Ok(());
        }

        Err(anyhow!("One or more server checks failed"))
    }

    fn ensure_app(&self, client: &SshClient, config: &AppConfig) -> Result<()> {
        let command = format!(
            "test -f {}/docker-compose.yml",
            shell_quote(&app_dir(config))
        );
        let (output, status) = client.execute_command(&command)?;
        if status != 0 {
            return Err(anyhow!(
                "No Minion app found at /opt/minion/{}. Run `minion deploy` first. {}",
                config.app_name,
                output
            ));
        }

        Ok(())
    }

    fn check(&self, client: &SshClient, label: &str, command: &str) -> Result<bool> {
        let (output, status) = client.execute_command(command)?;
        if status == 0 {
            println!("[ok] {}", label);
            return Ok(true);
        }

        println!("[fail] {}", label);
        if !output.trim().is_empty() {
            println!("{}", output.trim());
        }
        Ok(false)
    }
}

fn run_and_print(client: &SshClient, command: &str, error_message: &str) -> Result<()> {
    let (output, status) = client.execute_command(command)?;
    if !output.is_empty() {
        print!("{}", output);
    }

    if status != 0 {
        return Err(anyhow!("{}", error_message));
    }

    Ok(())
}

fn app_dir(config: &AppConfig) -> String {
    format!("/opt/minion/{}", config.app_name)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
