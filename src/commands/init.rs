use std::io::{self, Write};
use crate::utils::{Config, SshClient, CommandExecutor};
use std::path::Path;

// Include the resource files at compile time
const TRAEFIK_DOCKER_COMPOSE: &str = include_str!("../resources/docker-compose.yml");
const TRAEFIK_CONFIG_TEMPLATE: &str = include_str!("../resources/traefik.yml");

pub struct InitCommand;

impl InitCommand {
    pub fn new() -> Self {
        InitCommand
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

    fn setup_traefik(client: &SshClient, email: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Install Docker if not present
        println!("Checking Docker installation...");
        let (output, status) = client.execute_command("command -v docker || (curl -fsSL https://get.docker.com | sh)")?;
        if status == 0 {
            println!("✓ Docker is installed");
        } else {
            println!("Failed to install Docker: {}", output);
            return Err("Docker installation failed".into());
        }

        // Setup Traefik
        println!("Setting up Traefik...");
        let traefik_config = TRAEFIK_CONFIG_TEMPLATE.replace("{{email}}", email);

        let traefik_commands = [
            // Create directory structure
            "mkdir -p /opt/traefik/config/dynamic",
            "mkdir -p /opt/traefik/data",
            // Create and set permissions for acme.json
            "touch /opt/traefik/data/acme.json",
            "chmod 600 /opt/traefik/data/acme.json",
            // Create Docker network
            "docker network create traefik_proxy || true",
            // Write configuration files
            &format!("cat > /opt/traefik/config/traefik.yml << 'EOL'\n{}\nEOL", traefik_config),
            &format!("cat > /opt/traefik/docker-compose.yml << 'EOL'\n{}\nEOL", TRAEFIK_DOCKER_COMPOSE),
            // Start Traefik
            "cd /opt/traefik && docker compose up -d"
        ];

        for cmd in traefik_commands {
            let (output, status) = client.execute_command(cmd)?;
            if status != 0 {
                println!("Failed to execute command: {}", cmd);
                println!("Error: {}", output);
                return Err("Traefik setup failed".into());
            }
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
            // Disable root SSH login
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
        // Check if config file exists and read existing values
        let config = Config::new(".minion")?;
        let existing_host = config.get("VPS_HOST");
        let existing_email = config.get("CERT_EMAIL");

        // Prompt for host with existing value as default
        print!("Enter VPS hostname or IP address");
        if let Some(host_val) = &existing_host {
            print!(" [{}]", host_val);
        }
        print!(": ");
        io::stdout().flush()?;

        let mut input_host = String::new();
        io::stdin().read_line(&mut input_host)?;
        let input_host = input_host.trim();
        // Use existing value if user just pressed enter
        let host = if input_host.is_empty() {
            existing_host.ok_or("No existing host found and no input provided")?.to_string()
        } else {
            input_host.to_string()
        };

        // Prompt for email with existing value as default
        print!("Enter email address for SSL certificates");
        if let Some(email_val) = &existing_email {
            print!(" [{}]", email_val);
        }
        println!(":");
        io::stdout().flush()?;

        let mut input_email = String::new();
        io::stdin().read_line(&mut input_email)?;
        let input_email = input_email.trim();
        // Use existing value if user just pressed enter
        let email = if input_email.is_empty() {
            existing_email.ok_or("No existing email found and no input provided")?.to_string()
        } else {
            input_email.to_string()
        };

        // Save both configs
        let mut config = Config::new(".minion")?;
        config.set("VPS_HOST".to_string(), host.clone());
        config.set("CERT_EMAIL".to_string(), email.clone());
        config.save()?;

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

        // Connect as minion user and setup the server
        println!("Connecting as minion user...");
        let ssh_result = SshClient::connect(&host, "minion", None)
            .and_then(|client| {
                Self::setup_traefik(&client, &email)?;
                Ok(client)
            });

        match ssh_result {
            Ok(_client) => {
                println!("✓ SSH connection established!");
                println!("✓ Initialization complete!");
                Ok(())
            }
            Err(e) => {
                println!("✗ SSH connection failed: {}", e);
                Err(e)
            }
        }
    }
}