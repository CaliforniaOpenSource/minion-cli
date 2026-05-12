use crate::utils::{
    AppConfig, AppConfigOverrides, CommandExecutor, LocalCommandRunner, RemoteClient, SshClient,
};
use anyhow::{anyhow, Result};
use std::path::Path;
use std::rc::Rc;
use tempfile::Builder;

// Include the resource files at compile time
const APP_DOCKER_COMPOSE: &str = include_str!("../resources/docker-compose.app.yml");

pub struct DeployCommand {
    command_runner: Rc<dyn LocalCommandRunner>,
}

#[derive(Debug, Clone, Default)]
pub struct DeployOptions {
    pub yes: bool,
    pub ci: bool,
    pub overrides: AppConfigOverrides,
}

impl DeployCommand {
    pub fn new() -> Self {
        Self {
            command_runner: Rc::new(CommandExecutor::new()),
        }
    }

    #[cfg(test)]
    fn with_command_runner(command_runner: Rc<dyn LocalCommandRunner>) -> Self {
        Self { command_runner }
    }

    fn parse_volumes(volumes: &str) -> Result<Vec<(String, String)>> {
        let mut mappings = Vec::new();
        if volumes.is_empty() {
            return Ok(mappings);
        }

        for vol in volumes.split(',') {
            if let Some((local, remote)) = vol.split_once(':') {
                mappings.push((local.to_string(), remote.to_string()));
            } else {
                return Err(anyhow!("Invalid volume format: {}", vol));
            }
        }
        Ok(mappings)
    }

    fn render_compose(
        app_name: &str,
        urls: &str,
        port: &str,
        volume_mappings: &[String],
    ) -> Result<String> {
        let url_list: Vec<&str> = urls
            .split(',')
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .collect();
        if url_list.is_empty() {
            return Err(anyhow!("At least one URL must be provided"));
        }

        let host_rules = url_list
            .iter()
            .map(|url| format!("Host(`{}`)", url))
            .collect::<Vec<_>>()
            .join(" || ");

        let volumes_section = if volume_mappings.is_empty() {
            String::new()
        } else {
            format!("    volumes:\n{}", volume_mappings.join("\n"))
        };

        Ok(APP_DOCKER_COMPOSE
            .replace("{{app_name}}", app_name)
            .replace("{{host_rules}}", &host_rules)
            .replace("{{port}}", port)
            .replace("{{volumes_section}}", &volumes_section))
    }

    fn deploy_app(&self, client: &dyn RemoteClient, config: &AppConfig) -> Result<()> {
        let url_list: Vec<&str> = config
            .app_url
            .split(',')
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .collect();
        if url_list.is_empty() {
            return Err(anyhow!("At least one URL must be provided"));
        }
        let parsed_volumes = Self::parse_volumes(&config.app_volumes)?;

        // Build and save the Docker image
        println!("Building Docker image for {}...", config.docker_platform);
        let image_name = format!("minion_{}", config.app_name);
        let platform_arg = format!("--platform={}", config.docker_platform);

        // Build the image with platform specified
        let (output, status) = self
            .command_runner
            .execute("docker", &["build", "-t", &image_name, ".", &platform_arg])?;

        if status != 0 {
            return Err(anyhow!("Failed to build Docker image: {}", output));
        }

        println!("✓ Docker image built successfully");

        // Create a temporary file with .tar extension
        let temp_file = Builder::new().prefix("minion_").suffix(".tar").tempfile()?;
        let temp_path = temp_file.path().to_string_lossy().to_string();

        // Save the image to the temporary file
        println!("Saving image to temporary file...");
        let (output, status) = self
            .command_runner
            .execute("docker", &["save", "-o", &temp_path, &image_name])?;

        if status != 0 {
            return Err(anyhow!("Failed to save Docker image: {}", output));
        }

        println!("✓ Docker image saved to temporary file");

        println!("Creating app directory on VPS...");
        let app_dir = format!("/opt/minion/{}", config.app_name);
        let volumes_dir = format!("{}/volumes", app_dir);

        // Create directory and set permissions
        let setup_commands = [
            &format!("sudo mkdir -p {}", app_dir),
            &format!("sudo mkdir -p {}", volumes_dir),
            &format!("sudo chown -R minion:minion {}", app_dir),
        ];

        for cmd in setup_commands {
            let (output, status) = client.execute_command(cmd)?;
            if status != 0 {
                return Err(anyhow!("Failed to execute command {}: {}", cmd, output));
            }
        }

        // Process volumes
        let mut volume_mappings = Vec::new();

        if !parsed_volumes.is_empty() {
            println!("Processing volumes...");
            for (local_name, remote) in parsed_volumes {
                // Construct the full path on the VPS
                let vps_path = format!("{}/{}", volumes_dir, local_name);

                // Ensure the directory exists on the VPS
                let mkdir_cmd = format!("sudo mkdir -p {}", vps_path);
                let (output, status) = client.execute_command(&mkdir_cmd)?;
                if status != 0 {
                    return Err(anyhow!(
                        "Failed to create volume directory {}: {}",
                        vps_path,
                        output
                    ));
                }

                // Ensure permissions
                let chown_cmd = format!("sudo chown -R minion:minion {}", vps_path);
                client.execute_command(&chown_cmd)?;

                // Add to mappings
                volume_mappings.push(format!("      - {}:{}", vps_path, remote));
            }
        }

        // Generate and upload docker-compose file
        let compose_content = Self::render_compose(
            &config.app_name,
            &config.app_url,
            &config.app_port,
            &volume_mappings,
        )?;
        let compose_path = format!("{}/docker-compose.yml", app_dir);

        println!("Creating docker-compose.yml...");
        let write_compose = format!("cat > {} << 'EOL'\n{}\nEOL", compose_path, compose_content);

        let (output, status) = client.execute_command(&write_compose)?;
        if status != 0 {
            return Err(anyhow!("Failed to create docker-compose.yml: {}", output));
        }

        println!("✓ Docker compose file created");

        // Copy the image file to the VPS
        println!("Copying Docker image to VPS...");
        client.copy_file(&temp_path, &format!("{}/{}.tar", app_dir, config.app_name))?;

        // Load the image and clean up
        let deploy_commands = [
            &format!("cd {} && docker load -i {}.tar", app_dir, config.app_name),
            &format!("rm {}/{}.tar", app_dir, config.app_name),
            &format!("cd {} && docker compose up -d", app_dir),
        ];

        for cmd in deploy_commands {
            let (output, status) = client.execute_command(cmd)?;
            if status != 0 {
                return Err(anyhow!("Failed to execute command {}: {}", cmd, output));
            }
        }

        println!("✓ Application deployed successfully!");
        println!(
            "✓ Your app should be available at https://{} shortly",
            url_list[0]
        );
        Ok(())
    }

    pub fn execute(&self, options: DeployOptions) -> Result<()> {
        let interactive = !(options.yes || options.ci);
        let config = AppConfig::load(options.overrides, interactive, interactive)?;
        config.require_deploy()?;

        // Verify dockerfile before proceeding
        if !Path::new("Dockerfile").exists() {
            return Err(anyhow!("Dockerfile not found in current directory"));
        }
        println!("✓ Dockerfile found");

        // Connect to VPS and deploy
        println!("Connecting to {} as {}...", config.host, config.ssh_user);
        let client =
            SshClient::connect_with_auth(&config.host, &config.ssh_user, &config.ssh_auth())?;
        self.deploy_app(&client, &config)?;

        Ok(())
    }

    #[cfg(test)]
    fn execute_with_client(
        &self,
        options: DeployOptions,
        client: &dyn RemoteClient,
        project_dir: &Path,
    ) -> Result<()> {
        let interactive = !(options.yes || options.ci);
        let config = AppConfig::load(options.overrides, interactive, interactive)?;
        config.require_deploy()?;

        if !project_dir.join("Dockerfile").exists() {
            return Err(anyhow!("Dockerfile not found in current directory"));
        }

        self.deploy_app(client, &config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::test_support::{FakeLocalCommandRunner, FakeRemoteClient};

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

    fn command_with_runner(runner: std::rc::Rc<FakeLocalCommandRunner>) -> DeployCommand {
        DeployCommand::with_command_runner(runner)
    }

    fn compose_write_command(commands: &[String]) -> &str {
        commands
            .iter()
            .find(|command| command.contains("docker-compose.yml"))
            .expect("compose write command not found")
    }

    #[test]
    fn test_parse_volumes_empty() {
        let mappings = DeployCommand::parse_volumes("").unwrap();
        assert!(mappings.is_empty());
    }

    #[test]
    fn test_parse_volumes_valid() {
        let volumes = "my-vol:/remote/path,local:/other/remote";
        let mappings = DeployCommand::parse_volumes(volumes).unwrap();

        assert_eq!(mappings.len(), 2);
        assert_eq!(mappings[0].0, "my-vol");
        assert_eq!(mappings[0].1, "/remote/path");
        assert_eq!(mappings[1].0, "local");
        assert_eq!(mappings[1].1, "/other/remote");
    }

    #[test]
    fn test_parse_volumes_invalid_format() {
        let result = DeployCommand::parse_volumes("invalid_format");
        assert!(result.is_err());
    }

    #[test]
    fn successful_deploy_runs_local_and_remote_steps() {
        let runner = std::rc::Rc::new(FakeLocalCommandRunner::new());
        let command = command_with_runner(runner.clone());
        let remote = FakeRemoteClient::new();

        command.deploy_app(&remote, &app_config()).unwrap();

        let local_commands = runner.commands();
        assert_eq!(local_commands.len(), 2);
        assert_eq!(local_commands[0].command, "docker");
        assert_eq!(
            local_commands[0].args,
            vec![
                "build",
                "-t",
                "minion_my-app",
                ".",
                "--platform=linux/amd64"
            ]
        );
        assert_eq!(local_commands[1].command, "docker");
        assert_eq!(local_commands[1].args[0], "save");
        assert_eq!(local_commands[1].args[3], "minion_my-app");

        let remote_commands = remote.commands();
        assert!(remote_commands.contains(&"sudo mkdir -p /opt/minion/my-app".to_string()));
        assert!(remote_commands.contains(&"sudo mkdir -p /opt/minion/my-app/volumes".to_string()));
        assert!(
            remote_commands.contains(&"sudo chown -R minion:minion /opt/minion/my-app".to_string())
        );
        assert!(compose_write_command(&remote_commands).contains("Host(`app.example.com`)"));
        assert!(remote_commands
            .contains(&"cd /opt/minion/my-app && docker load -i my-app.tar".to_string()));
        assert!(remote_commands.contains(&"rm /opt/minion/my-app/my-app.tar".to_string()));
        assert!(
            remote_commands.contains(&"cd /opt/minion/my-app && docker compose up -d".to_string())
        );

        let copied_files = remote.copied_files();
        assert_eq!(copied_files.len(), 1);
        assert_eq!(copied_files[0].1, "/opt/minion/my-app/my-app.tar");
    }

    #[test]
    fn docker_build_failure_stops_before_remote_commands() {
        let runner = std::rc::Rc::new(FakeLocalCommandRunner::with_responses(vec![(
            "build failed",
            1,
        )]));
        let command = command_with_runner(runner.clone());
        let remote = FakeRemoteClient::new();

        let error = command.deploy_app(&remote, &app_config()).unwrap_err();

        assert!(error.to_string().contains("Failed to build Docker image"));
        assert_eq!(runner.commands().len(), 1);
        assert!(remote.commands().is_empty());
        assert!(remote.copied_files().is_empty());
    }

    #[test]
    fn docker_save_failure_stops_before_remote_copy() {
        let runner = std::rc::Rc::new(FakeLocalCommandRunner::with_responses(vec![
            ("", 0),
            ("save failed", 1),
        ]));
        let command = command_with_runner(runner.clone());
        let remote = FakeRemoteClient::new();

        let error = command.deploy_app(&remote, &app_config()).unwrap_err();

        assert!(error.to_string().contains("Failed to save Docker image"));
        assert_eq!(runner.commands().len(), 2);
        assert!(remote.commands().is_empty());
        assert!(remote.copied_files().is_empty());
    }

    #[test]
    fn missing_dockerfile_fails_before_local_or_remote_work() {
        let runner = std::rc::Rc::new(FakeLocalCommandRunner::new());
        let command = command_with_runner(runner.clone());
        let remote = FakeRemoteClient::new();
        let project = tempfile::tempdir().unwrap();

        let error = command
            .execute_with_client(
                DeployOptions {
                    ci: true,
                    overrides: AppConfigOverrides {
                        host: Some("example.com".to_string()),
                        app_name: Some("my-app".to_string()),
                        app_url: Some("app.example.com".to_string()),
                        app_port: Some("3000".to_string()),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                &remote,
                project.path(),
            )
            .unwrap_err();

        assert!(error.to_string().contains("Dockerfile not found"));
        assert!(runner.commands().is_empty());
        assert!(remote.commands().is_empty());
    }

    #[test]
    fn multiple_urls_render_traefik_host_rules() {
        let runner = std::rc::Rc::new(FakeLocalCommandRunner::new());
        let command = command_with_runner(runner);
        let remote = FakeRemoteClient::new();
        let mut config = app_config();
        config.app_url = "app.example.com,www.example.com".to_string();

        command.deploy_app(&remote, &config).unwrap();

        let remote_commands = remote.commands();
        assert!(compose_write_command(&remote_commands)
            .contains("Host(`app.example.com`) || Host(`www.example.com`)"));
    }

    #[test]
    fn empty_volumes_omit_compose_volumes_section() {
        let runner = std::rc::Rc::new(FakeLocalCommandRunner::new());
        let command = command_with_runner(runner);
        let remote = FakeRemoteClient::new();

        command.deploy_app(&remote, &app_config()).unwrap();

        let remote_commands = remote.commands();
        assert!(!compose_write_command(&remote_commands).contains("    volumes:\n"));
    }

    #[test]
    fn valid_volumes_create_remote_dirs_and_render_mappings() {
        let runner = std::rc::Rc::new(FakeLocalCommandRunner::new());
        let command = command_with_runner(runner);
        let remote = FakeRemoteClient::new();
        let mut config = app_config();
        config.app_volumes = "data:/data,uploads:/uploads".to_string();

        command.deploy_app(&remote, &config).unwrap();

        let remote_commands = remote.commands();
        assert!(
            remote_commands.contains(&"sudo mkdir -p /opt/minion/my-app/volumes/data".to_string())
        );
        assert!(remote_commands
            .contains(&"sudo mkdir -p /opt/minion/my-app/volumes/uploads".to_string()));

        let compose = compose_write_command(&remote_commands);
        assert!(compose.contains("      - /opt/minion/my-app/volumes/data:/data"));
        assert!(compose.contains("      - /opt/minion/my-app/volumes/uploads:/uploads"));
    }

    #[test]
    fn invalid_volume_syntax_fails_before_docker_build() {
        let runner = std::rc::Rc::new(FakeLocalCommandRunner::new());
        let command = command_with_runner(runner.clone());
        let remote = FakeRemoteClient::new();
        let mut config = app_config();
        config.app_volumes = "invalid".to_string();

        let error = command.deploy_app(&remote, &config).unwrap_err();

        assert!(error.to_string().contains("Invalid volume format"));
        assert!(runner.commands().is_empty());
        assert!(remote.commands().is_empty());
    }

    #[test]
    fn docker_platform_override_affects_build_command() {
        let runner = std::rc::Rc::new(FakeLocalCommandRunner::new());
        let command = command_with_runner(runner.clone());
        let remote = FakeRemoteClient::new();
        let mut config = app_config();
        config.docker_platform = "linux/arm64".to_string();

        command.deploy_app(&remote, &config).unwrap();

        assert_eq!(
            runner.commands()[0].args,
            vec![
                "build",
                "-t",
                "minion_my-app",
                ".",
                "--platform=linux/arm64"
            ]
        );
    }

    #[test]
    fn docker_platform_cli_override_affects_build_command() {
        let runner = std::rc::Rc::new(FakeLocalCommandRunner::new());
        let command = command_with_runner(runner.clone());
        let remote = FakeRemoteClient::new();
        let project = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("Dockerfile"), "FROM scratch\n").unwrap();

        command
            .execute_with_client(
                DeployOptions {
                    ci: true,
                    overrides: AppConfigOverrides {
                        host: Some("example.com".to_string()),
                        app_name: Some("my-app".to_string()),
                        app_url: Some("app.example.com".to_string()),
                        app_port: Some("3000".to_string()),
                        docker_platform: Some("linux/arm64".to_string()),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                &remote,
                project.path(),
            )
            .unwrap();

        assert_eq!(
            runner.commands()[0].args,
            vec![
                "build",
                "-t",
                "minion_my-app",
                ".",
                "--platform=linux/arm64"
            ]
        );
    }
}
