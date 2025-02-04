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

    fn load_args() -> anyhow::Result<(String, String, String, String)> {
        let config = Config::new(".minion")?;
        let existing_host = config.get("VPS_HOST");
        let existing_name = config.get("APP_NAME");
        let existing_url = config.get("APP_URL");
        let existing_port = config.get("APP_PORT");

        let host = Self::prompt_with_default(
            "Enter VPS hostname or IP address",
            existing_host
        )?;

        let name = Self::prompt_with_default(
            "Enter app name",
            existing_name
        )?;

        let url = Self::prompt_with_default(
            "Enter the URL for the app (e.g., app.example.com, subdomain.example.com, etc.)",
            existing_url
        )?;

        let port = Self::prompt_with_default(
            "Enter the port for the app (e.g., 8000)",
            existing_port
        )?;

        // Save both configs
        let mut config = Config::new(".minion")?;
        config.set("VPS_HOST".to_string(), host.clone());
        config.set("APP_NAME".to_string(), name.clone());
        config.set("APP_URL".to_string(), url.clone());
        config.set("APP_PORT".to_string(), port.clone());
        config.save()?;

        Ok((host, name, url, port))
    }

    pub fn execute(&self) -> anyhow::Result<()> {
        let (_host, _name, _url, _port) = Self::load_args()?;
        println!("✓ Configuration saved successfully!");
        Ok(())
    }
}