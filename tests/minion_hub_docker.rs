use std::path::Path;
use std::process::Command;

#[test]
#[ignore = "requires Docker, downloads Ubuntu/Rust packages, and can be slow"]
fn init_writes_config_files_idempotently_in_ubuntu_docker() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mount = format!("{}:/workspace:ro", repo.display());

    let output = Command::new("docker")
        .args([
            "run",
            "--rm",
            "-v",
            &mount,
            "-w",
            "/workspace",
            "ubuntu:25.10",
            "bash",
            "-euxc",
            DOCKER_TEST_SCRIPT,
        ])
        .output()
        .expect("failed to run docker");

    assert!(
        output.status.success(),
        "docker test failed\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

const DOCKER_TEST_SCRIPT: &str = r#"
export DEBIAN_FRONTEND=noninteractive

# Build and run from a plain Ubuntu image with only the packages needed to
# compile this crate. Rust comes from rustup so the test is not pinned to the
# Cargo version shipped by the base image.
# The init command uses --skip-system because this Docker test verifies
# generated files, not privileged systemd/WireGuard startup.
apt-get update
apt-get install -y --no-install-recommends \
  build-essential \
  ca-certificates \
  curl \
  libssh2-1-dev \
  libssl-dev \
  pkg-config
rm -rf /var/lib/apt/lists/*

export CARGO_HOME=/tmp/cargo-home
export RUSTUP_HOME=/tmp/rustup-home
export CARGO_TARGET_DIR=/tmp/minion-target
export PATH="${CARGO_HOME}/bin:${PATH}"
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
  | sh -s -- -y --profile minimal --default-toolchain stable --no-modify-path

cargo build --locked --bin minion-hub
cargo test --locked --bin minion-hub api_crud_updates_wireguard_and_coredns_files
cargo test --locked --bin minion-hub api_rejects_invalid_input_and_duplicate_ips

/tmp/minion-target/debug/minion-hub init --skip-system

test -f /etc/wireguard/minion0.conf
grep -F "[Interface]" /etc/wireguard/minion0.conf
grep -F "Address = 10.42.42.1/24" /etc/wireguard/minion0.conf
grep -F "ListenPort = 51820" /etc/wireguard/minion0.conf
grep -F "PrivateKey =" /etc/wireguard/minion0.conf

test -f /etc/coredns/Corefile
grep -F "bind 10.42.42.1" /etc/coredns/Corefile
grep -F "hosts /etc/coredns/minion.hosts" /etc/coredns/Corefile
grep -F "forward . /etc/resolv.conf" /etc/coredns/Corefile

test -f /etc/coredns/minion.hosts
test ! -s /etc/coredns/minion.hosts

test -f /etc/systemd/system/minion-hub.service
grep -F "After=network-online.target wg-quick@minion0.service" /etc/systemd/system/minion-hub.service
grep -F "Wants=network-online.target wg-quick@minion0.service" /etc/systemd/system/minion-hub.service
! grep -F "Requires=wg-quick@minion0.service" /etc/systemd/system/minion-hub.service

test -f /etc/systemd/system/coredns.service
grep -F "Requires=wg-quick@minion0.service" /etc/systemd/system/coredns.service

sha256sum \
  /etc/wireguard/minion0.conf \
  /etc/coredns/Corefile \
  /etc/coredns/minion.hosts \
  /etc/systemd/system/minion-hub.service \
  /etc/systemd/system/coredns.service \
  > /tmp/minion-hub-before.sha256

/tmp/minion-target/debug/minion-hub init --skip-system

sha256sum \
  /etc/wireguard/minion0.conf \
  /etc/coredns/Corefile \
  /etc/coredns/minion.hosts \
  /etc/systemd/system/minion-hub.service \
  /etc/systemd/system/coredns.service \
  > /tmp/minion-hub-after.sha256

cmp /tmp/minion-hub-before.sha256 /tmp/minion-hub-after.sha256
"#;
