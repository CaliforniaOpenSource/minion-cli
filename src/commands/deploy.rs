use crate::utils::{Config, SshClient};

pub struct DeployCommand;

impl DeployCommand {
    pub fn new() -> Self {
        DeployCommand
    }

    pub fn execute(&self) -> Result<(), Box<dyn std::error::Error>> {
        // Read the config file
        let config = Config::new(".minion")?;
        let host = config.get("VPS_HOST")
            .ok_or("VPS_HOST not found in .minion file. Please run 'minion init' first.")?;

        println!("Connecting to {}...", host);
        let client = SshClient::connect(host, "root", None)?;

        println!("Executing command...");
        let (output, status) = client.execute_command("echo \"Hello from minion\" > /tmp/minion")?;

        if status == 0 {
            println!("✓ Command executed successfully!");
        } else {
            println!("✗ Command failed with status code: {}", status);
            if !output.is_empty() {
                println!("Output: {}", output);
            }
        }

        Ok(())
    }
}