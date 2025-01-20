use std::io::{self, Write};
use crate::utils::{SshClient, CommandExecutor};
use std::path::Path;

// Include the resource files at compile time
const TRAEFIK_DOCKER_COMPOSE: &str = include_str!("../resources/docker-compose.traefik.yml");
const TRAEFIK_CONFIG_TEMPLATE: &str = include_str!("../resources/traefik.yml");

pub struct SetupCommand;

impl SetupCommand {
    pub fn new() -> Self {
        SetupCommand
    }

    fn check_dependencies() -> Result<(), Box<dyn std::error::Error>> {
        let cmd = CommandExecutor::new();

        // TODO: Add sops and age support

        // // Check if sops is installed
        // match cmd.execute("sops", &["--version"]) {
        //     Ok(_) => println!("✓ sops is installed"),
        //     Err(_) => return Err("sops is not installed. Please install it first.".into()),
        // }

        // // Check if age is installed
        // match cmd.execute("age", &["--version"]) {
        //     Ok(_) => println!("✓ age is installed"),
        //     Err(_) => return Err("age is not installed. Please install it first.".into()),
        // }

        // Check if docker is installed
        match cmd.execute("docker", &["--version"]) {
            Ok(_) => println!("✓ docker is installed"),
            Err(_) => return Err("docker is not installed. Please install it first.".into()),
        }

        Ok(())
    }

    fn _setup_keys() -> Result<(), Box<dyn std::error::Error>> {
        let home = std::env::var("HOME")?;
        let key_path = format!("{}/.config/sops/age/keys.txt", home);

        if !Path::new(&key_path).exists() {
            println!("Generating age key pair...");
            std::fs::create_dir_all(format!("{}/.config/sops/age", home))?;
            let (output, _) = CommandExecutor::new().execute("age-keygen", &["-o", &key_path])?;
            println!("✓ Generated age key pair: {}", output);
        } else {
            println!("✓ Age key pair already exists");
        }

        Ok(())
    }

    fn setup_docker(client: &SshClient) -> Result<(), Box<dyn std::error::Error>> {
        // Check if Docker is already installed
        println!("Checking if Docker is already installed...");
        let (docker_check, docker_status) = client.execute_command("command -v docker")?;

        if docker_status != 0 {
            println!("Docker not found, installing...");
            // Download Docker installation script
            let (_, download_status) = client.execute_command("curl -fsSL https://get.docker.com -o /tmp/get-docker.sh")?;
            if download_status != 0 {
                return Err("Failed to download Docker installation script".into());
            }

            // Make script executable and run it with sudo and capture full output
            let (install_output, install_status) = client.execute_command("sudo DEBIAN_FRONTEND=noninteractive sh /tmp/get-docker.sh 2>&1")?;
            println!("Docker installation output: {}", install_output);

            if install_status != 0 {
                return Err(format!("Docker installation failed with status {}: {}", install_status, install_output).into());
            }

            // Clean up
            let _ = client.execute_command("rm /tmp/get-docker.sh")?;

            // Add current user to docker group
            println!("Adding user to docker group...");
            let _ = client.execute_command("sudo usermod -aG docker $USER")?;

            println!("✓ Docker installed successfully");
        } else {
            println!("✓ Docker is already installed at: {}", docker_check.trim());
        }

        // Verify Docker is working
        let (version_output, version_status) = client.execute_command("docker --version")?;
        if version_status != 0 {
            return Err(format!("Docker verification failed: {}", version_output).into());
        }
        println!("✓ Docker version: {}", version_output.trim());

        Ok(())
    }

    fn setup_traefik(client: &SshClient, email: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Setup Traefik
        println!("Setting up Traefik...");

        // Verify docker compose is available
        println!("Verifying docker compose...");
        let (compose_output, compose_status) = client.execute_command("docker compose version")?;
        if compose_status != 0 {
            return Err(format!("Docker Compose not available: {}", compose_output).into());
        }
        println!("✓ Docker Compose is available");

        // Verify docker permissions
        println!("Verifying docker permissions...");
        let (groups_output, _) = client.execute_command("groups")?;
        if !groups_output.contains("docker") {
            return Err("Current user is not in the docker group. Please reconnect to the server.".into());
        }
        println!("✓ Docker permissions verified");

        let traefik_config = TRAEFIK_CONFIG_TEMPLATE.replace("{{email}}", email);

        let traefik_commands = [
            // Create directory structure
            "sudo mkdir -p /opt/traefik/config/dynamic",
            "sudo mkdir -p /opt/traefik/data",
            // Create and set permissions for acme.json
            "sudo touch /opt/traefik/data/acme.json",
            "sudo chmod 600 /opt/traefik/data/acme.json",
            // Create Docker network
            "docker network create traefik_proxy || true",
            // Write configuration files
            &format!("sudo bash -c 'cat > /opt/traefik/config/traefik.yml << EOL\n{}\nEOL'", traefik_config),
            &format!("sudo bash -c 'cat > /opt/traefik/docker-compose.yml << EOL\n{}\nEOL'", TRAEFIK_DOCKER_COMPOSE),
        ];

        for cmd in traefik_commands {
            let (output, status) = client.execute_command(cmd)?;
            if status != 0 {
                println!("Failed to execute command: {}", cmd);
                println!("Error: {}", output);
                return Err("Traefik setup failed".into());
            }
        }

        // Verify files exist
        println!("Verifying configuration files...");
        let (ls_output, ls_status) = client.execute_command("ls -l /opt/traefik/config/traefik.yml /opt/traefik/docker-compose.yml")?;
        if ls_status != 0 {
            return Err(format!("Configuration files not found: {}", ls_output).into());
        }
        println!("✓ Configuration files verified");

        // Start Traefik with detailed output
        println!("Starting Traefik...");
        let (compose_output, compose_status) = client.execute_command("cd /opt/traefik && docker compose up -d 2>&1")?;
        if compose_status != 0 {
            println!("Docker Compose output: {}", compose_output);
            return Err("Failed to start Traefik".into());
        }

        // Verify Traefik is running
        println!("Verifying Traefik is running...");
        let (ps_output, ps_status) = client.execute_command("docker ps --filter 'name=traefik' --format '{{.Status}}'")?;
        if ps_status != 0 || !ps_output.contains("Up") {
            return Err(format!("Traefik is not running. Status: {}", ps_output).into());
        }

        println!("✓ Traefik setup complete");
        Ok(())
    }

    fn setup_users(client: &SshClient) -> Result<(), Box<dyn std::error::Error>> {
        println!("✓ Root SSH connection successful!");

        // Create minion user if it doesn't exist
        let minion_setup_commands = [
            // Check if user exists, create if not
            "id -u minion &>/dev/null || useradd -m -s /bin/bash minion",
            // Setup SSH directory
            "mkdir -p /home/minion/.ssh",
            // Copy authorized keys
            "cp ~/.ssh/authorized_keys /home/minion/.ssh/",
            // Set proper ownership
            "chown -R minion:minion /home/minion/.ssh",
            // Set proper permissions
            "chmod 700 /home/minion/.ssh",
            "chmod 600 /home/minion/.ssh/authorized_keys",
            // Grant sudo privileges without password
            "echo 'minion ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/minion",
            "chmod 440 /etc/sudoers.d/minion",
            // Disable t SSH login
            "sed -i 's/^#\\?PermitRootLogin\\s*yes/PermitRootLogin no/' /etc/ssh/sshd_config",
            "sed -i 's/^#\\?PasswordAuthentication\\s*yes/PasswordAuthentication no/' /etc/ssh/sshd_config",
            // Restart SSH service to apply changes
            "systemctl restart ssh"
        ];

        for cmd in minion_setup_commands {
            println!("Executing: {}", cmd);
            let (output, status) = client.execute_command(cmd)?;
            if status != 0 {
                println!("✗ Command failed with status {}", status);
                if !output.is_empty() {
                    println!("Error output: {}", output);
                }
                return Err(format!("Failed to execute command: {}", cmd).into());
            }
            if !output.is_empty() {
                println!("Output: {}", output);
            }
        }
        println!("✓ Minion user setup complete");
        Ok(())
    }

    fn load_args() -> Result<(String, String), Box<dyn std::error::Error>> {
        print!("Enter VPS hostname or IP address: ");
        io::stdout().flush()?;

        let mut input_host = String::new();
        io::stdin().read_line(&mut input_host)?;
        let host = input_host.trim().to_string();

        print!("Enter email address for SSL certificates: ");
        io::stdout().flush()?;

        let mut input_email = String::new();
        io::stdin().read_line(&mut input_email)?;
        let email = input_email.trim().to_string();

        Ok((host, email))
    }

    pub fn execute(&self) -> Result<(), Box<dyn std::error::Error>> {
        // Check local dependencies first
        Self::check_dependencies()?;
        //Self::setup_keys()?; // TODO: Enable once sops and age are supported.

        let (host, email) = Self::load_args()?;

        println!("Testing SSH connection to {}...", host);

        // Try root first to ensure minion user exists
        if let Ok(client) = SshClient::connect(&host, "root", None) {
            Self::setup_users(&client)?;
        }

        // Connect as minion user and setup Docker
        println!("Connecting as minion user...");
        let client = SshClient::connect(&host, "minion", None)?;
        Self::setup_docker(&client)?;
        println!("Reconnecting to apply group changes...");
        drop(client);

        // Reconnect and setup Traefik
        let client = SshClient::connect(&host, "minion", None)?;
        Self::setup_traefik(&client, &email)?;

        println!("✓ SSH connection established!");
        println!("✓ Initialization complete!");
        Ok(())
    }
}