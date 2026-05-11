use anyhow::{anyhow, Result};
use std::env;
use std::io::{self, Write};

use super::{Config, SshAuth};

const CONFIG_FILE: &str = ".minion";

#[derive(Debug, Clone, Default)]
pub struct AppConfigOverrides {
    pub host: Option<String>,
    pub app_name: Option<String>,
    pub app_url: Option<String>,
    pub app_port: Option<String>,
    pub app_volumes: Option<String>,
    pub ssh_user: Option<String>,
    pub ssh_key_path: Option<String>,
    pub ssh_private_key: Option<String>,
    pub ssh_password: Option<String>,
    pub ssh_passphrase: Option<String>,
    pub docker_platform: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfig {
    pub host: String,
    pub app_name: String,
    pub app_url: String,
    pub app_port: String,
    pub app_volumes: String,
    pub ssh_user: String,
    pub ssh_key_path: Option<String>,
    pub ssh_private_key: Option<String>,
    pub ssh_password: Option<String>,
    pub ssh_passphrase: Option<String>,
    pub docker_platform: String,
}

impl AppConfig {
    pub fn load(
        overrides: AppConfigOverrides,
        interactive: bool,
        save_interactive: bool,
    ) -> Result<Self> {
        let config = Config::new(CONFIG_FILE)?;

        let mut host = pick(overrides.host, "MINION_VPS_HOST", &config, "VPS_HOST");
        let mut app_name = pick(overrides.app_name, "MINION_APP_NAME", &config, "APP_NAME");
        let mut app_url = pick(overrides.app_url, "MINION_APP_URL", &config, "APP_URL");
        let mut app_port = pick(overrides.app_port, "MINION_APP_PORT", &config, "APP_PORT");
        let mut app_volumes = pick(
            overrides.app_volumes,
            "MINION_APP_VOLUMES",
            &config,
            "APP_VOLUMES",
        );

        if interactive {
            host = Some(prompt_with_default(
                "Enter VPS hostname or IP address",
                host.as_deref(),
            )?);
            app_name = Some(prompt_with_default("Enter app name", app_name.as_deref())?);
            app_url = Some(prompt_with_default(
                "Enter the URL for the app (e.g., app.example.com)",
                app_url.as_deref(),
            )?);
            app_port = Some(prompt_with_default(
                "Enter the port for the app (e.g., 8000)",
                app_port.as_deref(),
            )?);
            app_volumes = Some(prompt_with_default(
                "Enter volume mappings (local:remote, comma separated)",
                app_volumes.as_deref(),
            )?);
        }

        let ssh_user = pick(overrides.ssh_user, "MINION_SSH_USER", &config, "SSH_USER")
            .unwrap_or_else(|| "minion".to_string());
        let ssh_key_path = pick(
            overrides.ssh_key_path,
            "MINION_SSH_KEY_PATH",
            &config,
            "SSH_KEY_PATH",
        );
        let ssh_private_key = pick_secret(overrides.ssh_private_key, "MINION_SSH_PRIVATE_KEY");
        let ssh_password = pick_secret(overrides.ssh_password, "MINION_SSH_PASSWORD");
        let ssh_passphrase = pick_secret(overrides.ssh_passphrase, "MINION_SSH_PASSPHRASE");
        let docker_platform = pick(
            overrides.docker_platform,
            "MINION_DOCKER_PLATFORM",
            &config,
            "DOCKER_PLATFORM",
        )
        .unwrap_or_else(|| "linux/amd64".to_string());

        let app_config = AppConfig {
            host: host.unwrap_or_default(),
            app_name: app_name.unwrap_or_default(),
            app_url: app_url.unwrap_or_default(),
            app_port: app_port.unwrap_or_default(),
            app_volumes: app_volumes.unwrap_or_default(),
            ssh_user,
            ssh_key_path,
            ssh_private_key,
            ssh_password,
            ssh_passphrase,
            docker_platform,
        };

        if interactive && save_interactive {
            app_config.save_app_file()?;
        }

        Ok(app_config)
    }

    pub fn require_deploy(&self) -> Result<()> {
        self.require_app_control()?;
        require_value("APP_URL", "MINION_APP_URL", &self.app_url)?;
        require_value("APP_PORT", "MINION_APP_PORT", &self.app_port)?;
        self.app_port_u16()?;
        Ok(())
    }

    pub fn require_app_control(&self) -> Result<()> {
        require_value("VPS_HOST", "MINION_VPS_HOST", &self.host)?;
        require_value("APP_NAME", "MINION_APP_NAME", &self.app_name)?;
        self.validate_app_name()?;
        Ok(())
    }

    pub fn app_port_u16(&self) -> Result<u16> {
        self.app_port
            .parse::<u16>()
            .map_err(|_| anyhow!("APP_PORT must be a valid TCP port"))
    }

    pub fn ssh_auth(&self) -> SshAuth {
        SshAuth {
            password: self.ssh_password.clone(),
            key_path: self.ssh_key_path.clone(),
            private_key: self.ssh_private_key.clone(),
            passphrase: self.ssh_passphrase.clone(),
        }
    }

    fn save_app_file(&self) -> Result<()> {
        let mut config = Config::new(CONFIG_FILE)?;
        config.set("VPS_HOST".to_string(), self.host.clone());
        config.set("APP_NAME".to_string(), self.app_name.clone());
        config.set("APP_URL".to_string(), self.app_url.clone());
        config.set("APP_PORT".to_string(), self.app_port.clone());
        config.set("APP_VOLUMES".to_string(), self.app_volumes.clone());
        config.save()?;
        Ok(())
    }

    fn validate_app_name(&self) -> Result<()> {
        if self.app_name.chars().all(is_safe_name_char) {
            return Ok(());
        }

        Err(anyhow!(
            "APP_NAME may only contain letters, numbers, dots, underscores, and dashes"
        ))
    }
}

fn pick(
    override_value: Option<String>,
    env_key: &str,
    config: &Config,
    config_key: &str,
) -> Option<String> {
    clean(override_value)
        .or_else(|| env_value(env_key))
        .or_else(|| {
            config
                .get(config_key)
                .cloned()
                .and_then(|value| clean(Some(value)))
        })
}

fn pick_secret(override_value: Option<String>, env_key: &str) -> Option<String> {
    clean_secret(override_value).or_else(|| clean_secret(env::var(env_key).ok()))
}

fn env_value(key: &str) -> Option<String> {
    env::var(key).ok().and_then(|value| clean(Some(value)))
}

fn clean(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn clean_secret(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.replace("\\n", "\n"))
        .filter(|value| !value.trim().is_empty())
}

fn prompt_with_default(prompt: &str, default: Option<&str>) -> Result<String> {
    print!("{}", prompt);
    if let Some(default_value) = default {
        print!(" [{}]", default_value);
    }
    print!(": ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();

    Ok(if input.is_empty() {
        default.unwrap_or_default().to_string()
    } else {
        input.to_string()
    })
}

fn require_value(config_key: &str, env_key: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(anyhow!(
            "{} is required. Set {} or run `minion init`.",
            config_key,
            env_key
        ));
    }

    Ok(())
}

fn is_safe_name_char(value: char) -> bool {
    value.is_ascii_alphanumeric() || value == '-' || value == '_' || value == '.'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_trims_and_rejects_empty_values() {
        assert_eq!(
            clean(Some(" value ".to_string())),
            Some("value".to_string())
        );
        assert_eq!(clean(Some("   ".to_string())), None);
        assert_eq!(clean(None), None);
    }

    #[test]
    fn clean_secret_preserves_multiline_values() {
        assert_eq!(
            clean_secret(Some("line1\\nline2".to_string())),
            Some("line1\nline2".to_string())
        );
    }

    #[test]
    fn deploy_config_requires_deploy_fields() {
        let config = AppConfig {
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
        };

        assert!(config.require_deploy().is_ok());
    }

    #[test]
    fn app_name_rejects_shell_unsafe_characters() {
        let config = AppConfig {
            host: "example.com".to_string(),
            app_name: "my-app;rm".to_string(),
            app_url: "app.example.com".to_string(),
            app_port: "3000".to_string(),
            app_volumes: String::new(),
            ssh_user: "minion".to_string(),
            ssh_key_path: None,
            ssh_private_key: None,
            ssh_password: None,
            ssh_passphrase: None,
            docker_platform: "linux/amd64".to_string(),
        };

        assert!(config.require_app_control().is_err());
    }
}
