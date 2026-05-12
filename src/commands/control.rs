use anyhow::{anyhow, Result};

use crate::utils::{AppConfig, AppConfigOverrides, RemoteClient, SshClient};

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

        self.execute_action(action, &config, &client)
    }

    fn execute_action(
        &self,
        action: ControlAction,
        config: &AppConfig,
        client: &dyn RemoteClient,
    ) -> Result<()> {
        match action {
            ControlAction::Status => self.status(client, config),
            ControlAction::Ps => self.ps(client, config),
            ControlAction::Logs { follow, tail } => self.logs(client, config, follow, tail),
            ControlAction::Restart => self.compose_action(client, config, "restart"),
            ControlAction::Stop => self.compose_action(client, config, "stop"),
            ControlAction::Start => self.compose_action(client, config, "up -d"),
            ControlAction::Doctor => self.doctor(client, config),
        }
    }

    fn status(&self, client: &dyn RemoteClient, config: &AppConfig) -> Result<()> {
        self.ensure_app(client, config)?;
        let command = format!(
            "cd {} && docker compose ps && printf '\\nRecent logs:\\n' && docker compose logs --tail 40 2>&1",
            shell_quote(&app_dir(config))
        );
        run_and_print(client, &command, "Failed to read app status")
    }

    fn ps(&self, client: &dyn RemoteClient, config: &AppConfig) -> Result<()> {
        self.ensure_app(client, config)?;
        let command = format!("cd {} && docker compose ps", shell_quote(&app_dir(config)));
        run_and_print(client, &command, "Failed to list app containers")
    }

    fn logs(
        &self,
        client: &dyn RemoteClient,
        config: &AppConfig,
        follow: bool,
        tail: u16,
    ) -> Result<()> {
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

    fn compose_action(
        &self,
        client: &dyn RemoteClient,
        config: &AppConfig,
        action: &str,
    ) -> Result<()> {
        self.ensure_app(client, config)?;
        let command = format!(
            "cd {} && docker compose {}",
            shell_quote(&app_dir(config)),
            action
        );
        run_and_print(client, &command, "Failed to control app")
    }

    fn doctor(&self, client: &dyn RemoteClient, config: &AppConfig) -> Result<()> {
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

    fn ensure_app(&self, client: &dyn RemoteClient, config: &AppConfig) -> Result<()> {
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

    fn check(&self, client: &dyn RemoteClient, label: &str, command: &str) -> Result<bool> {
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

fn run_and_print(client: &dyn RemoteClient, command: &str, error_message: &str) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::test_support::FakeRemoteClient;

    fn app_config() -> AppConfig {
        AppConfig {
            host: "example.com".to_string(),
            app_name: "my-app".to_string(),
            app_url: "app.example.com".to_string(),
            app_port: "3000".to_string(),
            app_volumes: String::new(),
            ssh_user: "minion".to_string(),
            ssh_key_path: None,
            ssh_private_key: None,
            ssh_password: None,
            ssh_passphrase: None,
            docker_platform: "linux/amd64".to_string(),
        }
    }

    #[test]
    fn status_checks_app_then_prints_ps_and_recent_logs() {
        let remote = FakeRemoteClient::new();

        ControlCommand::new()
            .execute_action(ControlAction::Status, &app_config(), &remote)
            .unwrap();

        let commands = remote.commands();
        assert_eq!(
            commands,
            vec![
                "test -f '/opt/minion/my-app'/docker-compose.yml",
                "cd '/opt/minion/my-app' && docker compose ps && printf '\\nRecent logs:\\n' && docker compose logs --tail 40 2>&1",
            ]
        );
    }

    #[test]
    fn ps_checks_app_then_runs_compose_ps() {
        let remote = FakeRemoteClient::new();

        ControlCommand::new()
            .execute_action(ControlAction::Ps, &app_config(), &remote)
            .unwrap();

        let commands = remote.commands();
        assert_eq!(
            commands,
            vec![
                "test -f '/opt/minion/my-app'/docker-compose.yml",
                "cd '/opt/minion/my-app' && docker compose ps",
            ]
        );
    }

    #[test]
    fn logs_defaults_to_tail_100() {
        let remote = FakeRemoteClient::new();

        ControlCommand::new()
            .execute_action(
                ControlAction::Logs {
                    follow: false,
                    tail: 100,
                },
                &app_config(),
                &remote,
            )
            .unwrap();

        assert_eq!(
            remote.commands(),
            vec!["test -f '/opt/minion/my-app'/docker-compose.yml"]
        );
        assert_eq!(
            remote.streamed_commands(),
            vec!["cd '/opt/minion/my-app' && docker compose logs --tail 100 2>&1"]
        );
    }

    #[test]
    fn logs_uses_requested_tail_and_follow_flag() {
        let remote = FakeRemoteClient::new();

        ControlCommand::new()
            .execute_action(
                ControlAction::Logs {
                    follow: true,
                    tail: 250,
                },
                &app_config(),
                &remote,
            )
            .unwrap();

        assert_eq!(
            remote.streamed_commands(),
            vec!["cd '/opt/minion/my-app' && docker compose logs --tail 250 --follow 2>&1"]
        );
    }

    #[test]
    fn logs_returns_error_when_stream_command_fails() {
        let remote = FakeRemoteClient::with_stream_responses(vec![1]);

        let error = ControlCommand::new()
            .execute_action(
                ControlAction::Logs {
                    follow: false,
                    tail: 100,
                },
                &app_config(),
                &remote,
            )
            .unwrap_err();

        assert!(error.to_string().contains("Failed to read app logs"));
    }

    #[test]
    fn restart_stop_and_start_run_expected_compose_actions() {
        for (action, expected) in [
            (ControlAction::Restart, "restart"),
            (ControlAction::Stop, "stop"),
            (ControlAction::Start, "up -d"),
        ] {
            let remote = FakeRemoteClient::new();

            ControlCommand::new()
                .execute_action(action, &app_config(), &remote)
                .unwrap();

            let commands = remote.commands();
            assert_eq!(
                commands[1],
                format!("cd '/opt/minion/my-app' && docker compose {}", expected)
            );
        }
    }

    #[test]
    fn missing_app_compose_file_returns_deploy_first_error() {
        let remote = FakeRemoteClient::with_responses(vec![("missing", 1)]);

        let error = ControlCommand::new()
            .execute_action(ControlAction::Ps, &app_config(), &remote)
            .unwrap_err();

        assert!(error.to_string().contains("Run `minion deploy` first"));
        assert_eq!(
            remote.commands(),
            vec!["test -f '/opt/minion/my-app'/docker-compose.yml"]
        );
    }

    #[test]
    fn doctor_checks_server_and_app_then_runs_ps_when_healthy() {
        let remote = FakeRemoteClient::new();

        ControlCommand::new()
            .execute_action(ControlAction::Doctor, &app_config(), &remote)
            .unwrap();

        assert_eq!(
            remote.commands(),
            vec![
                "docker --version",
                "docker compose version",
                "docker ps --filter 'name=traefik' --format '{{.Status}}' | grep -q Up",
                "test -f '/opt/minion/my-app'/docker-compose.yml",
                "test -f '/opt/minion/my-app'/docker-compose.yml",
                "cd '/opt/minion/my-app' && docker compose ps",
            ]
        );
    }

    #[test]
    fn doctor_fails_when_any_check_fails() {
        let remote = FakeRemoteClient::with_responses(vec![
            ("", 0),
            ("compose missing", 1),
            ("", 0),
            ("", 0),
        ]);

        let error = ControlCommand::new()
            .execute_action(ControlAction::Doctor, &app_config(), &remote)
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("One or more server checks failed"));
        assert_eq!(
            remote.commands(),
            vec![
                "docker --version",
                "docker compose version",
                "docker ps --filter 'name=traefik' --format '{{.Status}}' | grep -q Up",
                "test -f '/opt/minion/my-app'/docker-compose.yml",
            ]
        );
    }
}
