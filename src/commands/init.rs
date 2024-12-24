use std::io::{self, Write};
use crate::utils::{Config, SshClient};

pub struct InitCommand;

impl InitCommand {
    pub fn new() -> Self {
        InitCommand
    }

    pub fn execute(&self) -> Result<(), Box<dyn std::error::Error>> {
        print!("Enter VPS hostname or IP address: ");
        io::stdout().flush()?;

        let mut host = String::new();
        io::stdin().read_line(&mut host)?;
        let host = host.trim();

        // Save to .minion file
        let mut config = Config::new(".minion")?;
        config.set("VPS_HOST".to_string(), host.to_string());
        config.save()?;

        println!("Testing SSH connection to {}...", host);
        match SshClient::connect(host, "root", None) {
            Ok(_) => {
                println!("✓ SSH connection successful!");
                Ok(())
            }
            Err(e) => {
                println!("✗ SSH connection failed: {}", e);
                Err(e)
            }
        }
    }
}