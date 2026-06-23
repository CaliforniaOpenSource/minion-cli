use std::process::{Command, Output};

const MINION_ENV_KEYS: &[&str] = &[
    "MINION_VPS_HOST",
    "MINION_APP_NAME",
    "MINION_APP_URL",
    "MINION_APP_PORT",
    "MINION_APP_VOLUMES",
    "MINION_SSH_USER",
    "MINION_SSH_KEY_PATH",
    "MINION_SSH_PRIVATE_KEY",
    "MINION_SSH_PASSWORD",
    "MINION_SSH_PASSPHRASE",
    "MINION_DOCKER_PLATFORM",
];

fn minion_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_minion"));
    for key in MINION_ENV_KEYS {
        command.env_remove(key);
    }
    command
}

fn minion_hub_command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_minion-hub"))
}

fn run_in_empty_dir(args: &[&str]) -> Output {
    let temp_dir = tempfile::tempdir().unwrap();
    minion_command()
        .args(args)
        .current_dir(temp_dir.path())
        .output()
        .unwrap()
}

#[test]
fn root_help_lists_control_commands() {
    let output = minion_command().arg("--help").output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("deploy"));
    assert!(stdout.contains("status"));
    assert!(stdout.contains("logs"));
    assert!(stdout.contains("doctor"));
}

#[test]
fn minion_hub_help_is_available() {
    let output = minion_hub_command().arg("--help").output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("Hub companion executable for Minion"));
}

#[test]
fn deploy_help_lists_ci_and_ssh_options() {
    let output = minion_command()
        .args(["deploy", "--help"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("--ci"));
    assert!(stdout.contains("--ssh-key-path"));
    assert!(stdout.contains("--ssh-private-key"));
    assert!(stdout.contains("--docker-platform"));
}

#[test]
fn logs_help_lists_tail_and_follow_options() {
    let output = minion_command().args(["logs", "--help"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("--tail"));
    assert!(stdout.contains("--follow"));
}

#[test]
fn deploy_ci_without_config_fails_on_missing_host() {
    let output = run_in_empty_dir(&["deploy", "--ci"]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("VPS_HOST is required"));
}

#[test]
fn status_without_config_fails_on_missing_host() {
    let output = run_in_empty_dir(&["status"]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("VPS_HOST is required"));
}

#[test]
fn invalid_status_app_name_fails_before_ssh() {
    let output = run_in_empty_dir(&["status", "--host", "example.com", "--app", "bad;name"]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("APP_NAME may only contain"));
}
