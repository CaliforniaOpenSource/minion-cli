use std::process::Command;

pub struct CommandExecutor;

impl CommandExecutor {
    pub fn new() -> Self {
        CommandExecutor
    }

    pub fn execute(&self, command: &str, args: &[&str]) -> Result<(String, i32), Box<dyn std::error::Error>> {
        let output = Command::new(command)
            .args(args)
            .output()?;

        let stdout = String::from_utf8(output.stdout)?;
        let exit_status = output.status.code().unwrap_or(-1);

        Ok((stdout, exit_status))
    }
}