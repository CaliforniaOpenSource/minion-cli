use ssh2::Session;
use std::net::TcpStream;
use std::io::Read;

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
}