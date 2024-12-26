use ssh2::Session;
use std::net::TcpStream;
use std::io::Read;
use std::path::Path;
use std::fs::File;
use std::io::Write;

pub struct SshClient {
    session: Session,
}

impl SshClient {
    pub fn connect(host: &str, username: &str, password: Option<&str>) -> Result<Self, Box<dyn std::error::Error>> {
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

    pub fn execute_command(&self, command: &str) -> Result<(String, i32), Box<dyn std::error::Error>> {
        let mut channel = self.session.channel_session()?;
        channel.exec(command)?;

        let mut output = String::new();
        channel.read_to_string(&mut output)?;

        channel.wait_close()?;
        let exit_status = channel.exit_status()?;

        Ok((output, exit_status))
    }

    pub fn copy_file(&self, local_path: &str, remote_path: &str) -> Result<(), Box<dyn std::error::Error>> {
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