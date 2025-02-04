use crate::utils::{CommandExecutor, Config, SshClient};
use anyhow::{anyhow, Result};
use std::io::{self, Write};
use std::path::Path;
use tempfile::Builder;

// Include the resource files at compile time
const APP_DOCKER_COMPOSE: &str = include_str!("../resources/docker-compose.app.yml");

pub struct DeployCommand;

impl DeployCommand {
    pub fn new() -> Self {
        DeployCommand
    }

    fn deploy_app(&self, client: &SshClient, app_name: &str, urls: &str, port: u16) -> Result<()> {
        let url_list: Vec<&str> = urls.split(',').collect();
        if url_list.is_empty() {
            return Err(anyhow!("At least one URL must be provided"));
        }

        // Build and save the Docker image
        println!("Building Docker image for ARM64...");
        let cmd = CommandExecutor::new();

        // Build the image with platform specified
        let (output, status) = cmd.execute(
            "docker",
            &[
                "build",
                "-t",
                &format!("minion_{}", app_name),
                ".",
                "--platform=linux/amd64",
            ],
        )?;

        if status != 0 {
            return Err(anyhow!("Failed to build Docker image: {}", output));
        }

        println!("✓ Docker image built successfully");

        // Create a temporary file with .tar extension
        let temp_file = Builder::new()
            .prefix("minion_")
            .suffix(".tar")
            .tempfile()?;
        let temp_path = temp_file.path().to_string_lossy().to_string();

        // Save the image to the temporary file
        println!("Saving image to temporary file...");
        let (output, status) = cmd.execute(
            "docker",
            &[
                "save",
                "-o",
                &temp_path,
                &format!("minion_{}", app_name),
            ],
        )?;

        if status != 0 {
            return Err(anyhow!("Failed to save Docker image: {}", output));
        }

        println!("✓ Docker image saved to temporary file");

        println!("Creating app directory on VPS...");
        let app_dir = format!("/opt/minion/{}", app_name);

        // Create directory and set permissions
        let setup_commands = [
            &format!("sudo mkdir -p {}", app_dir),
            &format!("sudo chown minion:minion {}", app_dir),
        ];

        for cmd in setup_commands {
            let (output, status) = client.execute_command(cmd)?;
            if status != 0 {
                return Err(anyhow!("Failed to execute command {}: {}", cmd, output));
            }
        }

        let host_rules = url_list
            .iter()
            .map(|url| format!("Host(`{}`)", url))
            .collect::<Vec<_>>()
            .join(" || ");

        // Generate and upload docker-compose file
        let compose_content = APP_DOCKER_COMPOSE
            .replace("{{app_name}}", app_name)
            .replace("{{host_rules}}", &host_rules)
            .replace("{{port}}", &port.to_string());
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
        client.copy_file(
            &temp_path,
            &format!("{}/{}.tar", app_dir, app_name),
        )?;

        // Load the image and clean up
        let deploy_commands = [
            &format!("cd {} && docker load -i {}.tar", app_dir, app_name),
            &format!("rm {}/{}.tar", app_dir, app_name),
            &format!("cd {} && docker compose up -d", app_dir),
        ];

        for cmd in deploy_commands {
            let (output, status) = client.execute_command(cmd)?;
            if status != 0 {
                return Err(anyhow!("Failed to execute command {}: {}", cmd, output));
            }
        }

        println!("✓ Application deployed successfully!");
        println!("✓ Your app should be available at https://{} shortly", url_list[0]);
        Ok(())
    }

    fn prompt_with_default(prompt: &str, default: Option<&String>) -> anyhow::Result<String> {
        print!("{}", prompt);
        if let Some(default_val) = default {
            print!(" [{}]", default_val);
        }
        print!(": ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim();

        Ok(if input.is_empty() {
            default.expect("No default value provided").to_string()
        } else {
            input.to_string()
        })
    }

    fn load_args() -> anyhow::Result<(String, String, String, String)> {
        let config = Config::new(".minion")?;
        let existing_host = config.get("VPS_HOST");
        let existing_name = config.get("APP_NAME");
        let existing_url = config.get("APP_URL");
        let existing_port = config.get("APP_PORT");

        let host = Self::prompt_with_default("Enter VPS hostname or IP address", existing_host)?;

        let name = Self::prompt_with_default("Enter app name", existing_name)?;

        let url = Self::prompt_with_default(
            "Enter the URL for the app (e.g., app.example.com)",
            existing_url,
        )?;

        let port =
            Self::prompt_with_default("Enter the port for the app (e.g., 8000)", existing_port)?;

        // Save both configs
        let mut config = Config::new(".minion")?;
        config.set("VPS_HOST".to_string(), host.clone());
        config.set("APP_NAME".to_string(), name.clone());
        config.set("APP_URL".to_string(), url.clone());
        config.set("APP_PORT".to_string(), port.clone());
        config.save()?;

        Ok((host, name, url, port))
    }

    pub fn execute(&self) -> Result<()> {
        // Get all arguments up front
        let (host, app_name, url, port) = Self::load_args()?;
        let port = port.parse::<u16>()?;

        println!("Connecting to {}...", host);

        // Verify dockerfile before proceeding
        if !Path::new("Dockerfile").exists() {
            return Err(anyhow!("Dockerfile not found in current directory"));
        }
        println!("✓ Dockerfile found");

        // Connect to VPS and deploy
        let client = SshClient::connect(&host, "minion", None)?;
        self.deploy_app(&client, &app_name, &url, port)?;

        Ok(())
    }
}
