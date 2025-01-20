use ssh2::Session;
use std::net::TcpStream;
use std::io::Read;
use std::path::Path;
use std::fs::File;
use std::io::Write;
use anyhow::Result;

pub struct SshClient {
    session: Session,
}

impl SshClient {
    pub fn connect(host: &str, username: &str, password: Option<&str>) -> Result<Self> {
        let host_with_port = if host.contains(":") {
            host.to_string()
        } else {
            format!("{}:22", host)
        };

        let tcp = TcpStream::connect(host_with_port)?;
        let mut session = Session::new()?;
        session.set_tcp_stream(tcp);
        session.handshake()?;

        if let Some(pass) = password {
            session.userauth_password(username, pass)?;
        } else {
            session.userauth_agent(username)?;
        }

        Ok(SshClient { session })
    }

    pub fn execute_command(&self, command: &str) -> Result<(String, i32)> {
        let mut channel = self.session.channel_session()?;
        channel.exec(command)?;

        let mut output = String::new();
        channel.read_to_string(&mut output)?;

        channel.wait_close()?;
        let exit_status = channel.exit_status()?;

        Ok((output, exit_status))
    }

    pub fn copy_file(&self, local_path: &str, remote_path: &str) -> Result<()> {
        let mut local_file = File::open(local_path)?;
        let mut contents = Vec::new();
        local_file.read_to_end(&mut contents)?;

        let mut remote_file = self.session.scp_send(
            Path::new(remote_path),
            0o644,
            contents.len() as u64,
            None,
        )?;

        remote_file.write_all(&contents)?;
        remote_file.send_eof()?;
        remote_file.wait_eof()?;
        remote_file.close()?;
        remote_file.wait_close()?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;
    use std::collections::HashMap;

    use testcontainers::{
        core::{IntoContainerPort, WaitFor, ContainerPort},
        runners::SyncRunner,
        Image,
    };

    pub struct OpenSshServerContainer {
        env_vars: HashMap<String, String>,
        exposed_ports: Vec<ContainerPort>,
    }

    impl Default for OpenSshServerContainer {
        fn default() -> Self {
            let mut env_vars = HashMap::new();
            env_vars.insert("PASSWORD_ACCESS".to_string(), "true".to_string());
            env_vars.insert("USER_NAME".to_string(), "testuser".to_string());
            env_vars.insert("USER_PASSWORD".to_string(), "testpass".to_string());
            let exposed_ports = vec![2222.tcp()];
            Self { env_vars, exposed_ports }
        }
    }

    impl Image for OpenSshServerContainer {

        fn name(&self) -> &str {
            "linuxserver/openssh-server"
        }

        fn tag(&self) -> &str {
            "latest"
        }

        fn ready_conditions(&self) -> Vec<WaitFor> {
            vec![WaitFor::message_on_stdout("[ls.io-init] done.")]
        }

        fn expose_ports(&self) -> &[ContainerPort] {
            &self.exposed_ports
        }

        fn env_vars(&self) -> impl IntoIterator<Item = (impl Into<Cow<'_, str>>, impl Into<Cow<'_, str>>)> {
            Box::new(self.env_vars.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        }
    }

    #[test]
    fn test_ssh_connection_and_commands() {
        let image = OpenSshServerContainer::default();
        let container = image.start().unwrap();

        let port = container.get_host_port_ipv4(2222).unwrap();
        let client = SshClient::connect(
            &format!("localhost:{}", port),
            "testuser",
            Some("testpass")
        ).expect("Failed to connect");

        // Test command execution
        let (output, status) = client.execute_command("echo 'hello world'")
            .expect("Failed to execute command");
        assert_eq!(status, 0);
        assert_eq!(output.trim(), "hello world");

        // // Test file copy
        // // TODO: Add file copy test
    }
}