use std::io::{self, Write};
use crate::utils::{Config};
use anyhow;

pub struct InitCommand;

impl InitCommand {
    pub fn new() -> Self {
        InitCommand
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

    fn load_args() -> anyhow::Result<(String, String)> {
        let config = Config::new(".minion")?;
        let existing_host = config.get("VPS_HOST");
        let existing_email = config.get("CERT_EMAIL");

        let host = Self::prompt_with_default(
            "Enter VPS hostname or IP address",
            existing_host
        )?;

        let email = Self::prompt_with_default(
            "Enter email address for SSL certificates",
            existing_email
        )?;

        // Save both configs
        let mut config = Config::new(".minion")?;
        config.set("VPS_HOST".to_string(), host.clone());
        config.set("CERT_EMAIL".to_string(), email.clone());
        config.save()?;

        Ok((host, email))
    }

    pub fn execute(&self) -> anyhow::Result<()> {
        let (_host, _email) = Self::load_args()?;
        println!("✓ Configuration saved successfully!");
        Ok(())
    }
}