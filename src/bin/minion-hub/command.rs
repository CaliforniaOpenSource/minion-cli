//! Local command execution for provisioning and runtime reconciliation.

use anyhow::{bail, Context, Result};
use std::io::Write;
use std::process::{Command, Stdio};

pub(crate) trait CommandRunner {
    fn enabled(&self) -> bool;
    fn run(&self, program: &str, args: &[&str]) -> Result<String>;
    fn run_with_stdin(&self, program: &str, args: &[&str], stdin: &str) -> Result<String>;
}

pub(crate) struct SystemCommandRunner {
    skip_system: bool,
}

impl SystemCommandRunner {
    pub(crate) fn new(skip_system: bool) -> Self {
        Self { skip_system }
    }
}

impl CommandRunner for SystemCommandRunner {
    fn enabled(&self) -> bool {
        !self.skip_system
    }

    fn run(&self, program: &str, args: &[&str]) -> Result<String> {
        if self.skip_system {
            return Ok(String::new());
        }

        let output = Command::new(program)
            .args(args)
            .output()
            .with_context(|| format!("failed to run {}", program))?;
        command_output(program, args, output)
    }

    fn run_with_stdin(&self, program: &str, args: &[&str], stdin: &str) -> Result<String> {
        if self.skip_system {
            return Ok(String::new());
        }

        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to run {}", program))?;

        child
            .stdin
            .as_mut()
            .context("failed to open command stdin")?
            .write_all(stdin.as_bytes())?;

        let output = child.wait_with_output()?;
        command_output(program, args, output)
    }
}

fn command_output(program: &str, args: &[&str], output: std::process::Output) -> Result<String> {
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        bail!(
            "{} {} failed: {}{}",
            program,
            args.join(" "),
            stdout,
            stderr
        );
    }
    Ok(stdout)
}
