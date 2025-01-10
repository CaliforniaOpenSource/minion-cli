use std::io::{self, Write};
use std::path::Path;
use crate::utils::{Config, SshClient, CommandExecutor};

pub struct DeployCommand;

impl DeployCommand {
    pub fn new() -> Self {
        DeployCommand
    }

    fn load_or_prompt_app_name(config: &mut Config) -> Result<String, Box<dyn std::error::Error>> {
        let existing_app = config.get("APP_NAME");

        print!("Enter app name");
        if let Some(app_val) = &existing_app {
            print!(" [{}]", app_val);
        }
        print!(": ");
        io::stdout().flush()?;

        let mut input_app = String::new();
        io::stdin().read_line(&mut input_app)?;
        let input_app = input_app.trim();

        let app_name = if input_app.is_empty() {
            existing_app.ok_or("No existing app name found and no input provided")?.to_string()
        } else {
            input_app.to_string()
        };

        config.set("APP_NAME".to_string(), app_name.clone());
        config.save()?;

        Ok(app_name)
    }

    fn verify_dockerfile() -> Result<(), Box<dyn std::error::Error>> {
        if !Path::new("Dockerfile").exists() {
            return Err("Dockerfile not found in current directory".into());
        }
        println!("✓ Dockerfile found");
        Ok(())
    }

    fn prompt_app_url(config: &mut Config) -> Result<String, Box<dyn std::error::Error>> {
        let existing_url = config.get("APP_URL");

        print!("Enter the URL for the app (e.g., app.example.com)");
        if let Some(url_val) = &existing_url {
            print!(" [{}]", url_val);
        }
        print!(": ");
        io::stdout().flush()?;

        let mut input_url = String::new();
        io::stdin().read_line(&mut input_url)?;
        let input_url = input_url.trim();

        let url = if input_url.is_empty() && existing_url.is_some() {
            existing_url.unwrap().to_string()
        } else {
            input_url.to_string()
        };

        config.set("APP_URL".to_string(), url.clone());
        config.save()?;

        Ok(url)
    }

    fn prompt_app_port(config: &mut Config) -> Result<u16, Box<dyn std::error::Error>> {
        let existing_port = config.get("APP_PORT");

        print!("Enter the port your app exposes inside the container");
        if let Some(port_val) = &existing_port {
            print!(" [{}]", port_val);
        }
        print!(": ");
        io::stdout().flush()?;

        let mut input_port = String::new();
        io::stdin().read_line(&mut input_port)?;
        let input_port = input_port.trim();

        let port: u16 = if input_port.is_empty() && existing_port.is_some() {
            existing_port.unwrap().parse()?
        } else {
            input_port.parse()?
        };

        config.set("APP_PORT".to_string(), port.to_string());
        config.save()?;

        Ok(port)
    }

    fn build_and_save_image(&self, app_name: &str) -> Result<(), Box<dyn std::error::Error>> {
        println!("Building Docker image for ARM64...");
        let cmd = CommandExecutor::new();

        // Build the image with platform specified
        let (output, status) = cmd.execute("docker", &[
            "build",
            "-t",
            &format!("minion_{}", app_name),
            ".",
            "--platform=linux/amd64",
        ])?;

        if status != 0 {
            return Err(format!("Failed to build Docker image: {}", output).into());
        }

        println!("✓ Docker image built successfully");

        // Save the image to a tar file
        println!("Saving image to file...");
        let (output, status) = cmd.execute("docker", &[
            "save",
            "-o",
            &format!("{}.tar", app_name),
            &format!("minion_{}", app_name)
        ])?;

        if status != 0 {
            return Err(format!("Failed to save Docker image: {}", output).into());
        }

        println!("✓ Docker image saved to {}.tar", app_name);
        Ok(())
    }

    fn generate_compose_file(app_name: &str, url: &str, port: u16) -> String {
        format!(r#"
services:
  {app_name}:
    image: minion_{app_name}
    restart: unless-stopped
    networks:
      - traefik_network
    labels:
      - "traefik.enable=true"
      - "traefik.http.routers.{app_name}.rule=Host(`{url}`)"
      - "traefik.http.routers.{app_name}.entrypoints=websecure"
      - "traefik.http.routers.{app_name}.tls.certresolver=letsencrypt"
      - "traefik.http.services.{app_name}.loadbalancer.server.port={port}"

networks:
  traefik_network:
    external: true
"#)
    }

    fn deploy_app(&self, client: &SshClient, app_name: &str, url: &str, port: u16) -> Result<(), Box<dyn std::error::Error>> {
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
                return Err(format!("Failed to execute command {}: {}", cmd, output).into());
            }
        }

        // Generate and upload docker-compose file
        let compose_content = Self::generate_compose_file(app_name, url, port);
        let compose_path = format!("{}/docker-compose.yml", app_dir);

        println!("Creating docker-compose.yml...");
        let write_compose = format!(
            "cat > {} << 'EOL'\n{}\nEOL",
            compose_path,
            compose_content
        );

        let (output, status) = client.execute_command(&write_compose)?;
        if status != 0 {
            return Err(format!("Failed to create docker-compose.yml: {}", output).into());
        }

        println!("✓ Docker compose file created");

        // Copy the image file to the VPS
        println!("Copying Docker image to VPS...");
        client.copy_file(&format!("{}.tar", app_name), &format!("{}/{}.tar", app_dir, app_name))?;

        // Load the image and clean up
        let deploy_commands = [
            &format!("cd {} && docker load -i {}.tar", app_dir, app_name),
            &format!("rm {}/{}.tar", app_dir, app_name),
            &format!("cd {} && docker compose up -d", app_dir),
        ];

        for cmd in deploy_commands {
            let (output, status) = client.execute_command(cmd)?;
            if status != 0 {
                return Err(format!("Failed to execute command {}: {}", cmd, output).into());
            }
        }

        // Clean up local tar file
        std::fs::remove_file(format!("{}.tar", app_name))?;

        println!("✓ Application deployed successfully!");
        println!("✓ Your app should be available at https://{} shortly", url);
        Ok(())
    }

    fn load_args(config: &mut Config) -> Result<(String, String, String, u16), Box<dyn std::error::Error>> {
        // Get host first (immutable read)
        let host = config.get("VPS_HOST")
            .ok_or("VPS host not found in config")?
            .to_string();

        // Get app name with prompt
        let app_name = Self::load_or_prompt_app_name(config)?;

        // Get URL and port with defaults
        let url = Self::prompt_app_url(config)?;
        let port = Self::prompt_app_port(config)?;

        Ok((host, app_name, url, port))
    }

    pub fn execute(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut config = Config::new(".minion")?;

        // Get all arguments up front
        let (host, app_name, url, port) = Self::load_args(&mut config)?;

        println!("Connecting to {}...", host);

        // Verify dockerfile before proceeding
        Self::verify_dockerfile()?;

        // Build and save the Docker image
        self.build_and_save_image(&app_name)?;

        // Connect to VPS and deploy
        let client = SshClient::connect(&host, "minion", None)?;
        self.deploy_app(&client, &app_name, &url, port)?;

        Ok(())
    }
}