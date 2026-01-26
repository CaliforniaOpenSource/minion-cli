use std::process::Command;
use anyhow::Result;

pub struct CommandExecutor;

impl CommandExecutor {
    pub fn new() -> Self {
        CommandExecutor
    }

    pub fn execute(&self, command: &str, args: &[&str]) -> Result<(String, i32)> {
        let output = Command::new(command)
            .args(args)
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_status = output.status.code().unwrap_or(-1);

        let combined_output = format!("{}{}", stdout, stderr);

        Ok((combined_output, exit_status))
    }
}