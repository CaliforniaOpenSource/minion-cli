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

    fn deploy_app(&self, client: &SshClient, app_name: &str, urls: &str, port: u16, volumes: &str) -> Result<()> {
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
        let parsed_volumes = Self::parse_volumes(volumes)?;
        
        if !parsed_volumes.is_empty() {
            println!("Processing volumes...");
            for (local_name, remote) in parsed_volumes {
                // Construct the full path on the VPS
                let vps_path = format!("{}/{}", volumes_dir, local_name);
                
                // Ensure the directory exists on the VPS
                let mkdir_cmd = format!("sudo mkdir -p {}", vps_path);
                let (output, status) = client.execute_command(&mkdir_cmd)?;
                if status != 0 {
                    return Err(anyhow!("Failed to create volume directory {}: {}", vps_path, output));
                }

                // Ensure permissions
                let chown_cmd = format!("sudo chown -R minion:minion {}", vps_path);
                client.execute_command(&chown_cmd)?;

                // Add to mappings
                volume_mappings.push(format!(
                    "      - {}:{}",
                    vps_path, remote
                ));
            }
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

        // Generate and upload docker-compose file
        let compose_content = APP_DOCKER_COMPOSE
            .replace("{{app_name}}", app_name)
            .replace("{{host_rules}}", &host_rules)
            .replace("{{port}}", &port.to_string())
            .replace("{{volumes_section}}", &volumes_section);
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
            default.cloned().unwrap_or_default()
        } else {
            input.to_string()
        })
    }

    fn load_args() -> anyhow::Result<(String, String, String, String, String)> {
        let config = Config::new(".minion")?;
        let existing_host = config.get("VPS_HOST");
        let existing_name = config.get("APP_NAME");
        let existing_url = config.get("APP_URL");
        let existing_port = config.get("APP_PORT");
        let existing_volumes = config.get("APP_VOLUMES");

        let host = Self::prompt_with_default("Enter VPS hostname or IP address", existing_host)?;

        let name = Self::prompt_with_default("Enter app name", existing_name)?;

        let url = Self::prompt_with_default(
            "Enter the URL for the app (e.g., app.example.com)",
            existing_url,
        )?;

        let port =
            Self::prompt_with_default("Enter the port for the app (e.g., 8000)", existing_port)?;

        let volumes = Self::prompt_with_default(
            "Enter volume mappings (local:remote, comma separated)",
            existing_volumes,
        )?;

        // Save both configs
        let mut config = Config::new(".minion")?;
        config.set("VPS_HOST".to_string(), host.clone());
        config.set("APP_NAME".to_string(), name.clone());
        config.set("APP_URL".to_string(), url.clone());
        config.set("APP_PORT".to_string(), port.clone());
        config.set("APP_VOLUMES".to_string(), volumes.clone());
        config.save()?;

        Ok((host, name, url, port, volumes))
    }

    pub fn execute(&self) -> Result<()> {
        // Get all arguments up front
        let (host, app_name, url, port, volumes) = Self::load_args()?;
        let port = port.parse::<u16>()?;

        println!("Connecting to {}...", host);

        // Verify dockerfile before proceeding
        if !Path::new("Dockerfile").exists() {
            return Err(anyhow!("Dockerfile not found in current directory"));
        }
        println!("✓ Dockerfile found");

        // Connect to VPS and deploy
        let client = SshClient::connect(&host, "minion", None)?;
        self.deploy_app(&client, &app_name, &url, port, &volumes)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
