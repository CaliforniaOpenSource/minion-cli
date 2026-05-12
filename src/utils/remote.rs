use anyhow::Result;

pub trait RemoteClient {
    fn execute_command(&self, command: &str) -> Result<(String, i32)>;
    fn execute_command_stream(&self, command: &str) -> Result<i32>;
    fn copy_file(&self, local_path: &str, remote_path: &str) -> Result<()>;
}
