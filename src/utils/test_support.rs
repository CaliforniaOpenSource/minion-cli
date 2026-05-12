use anyhow::Result;
use std::cell::RefCell;
use std::collections::VecDeque;

use super::{LocalCommandRunner, RemoteClient};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalCommandInvocation {
    pub command: String,
    pub args: Vec<String>,
}

#[derive(Default)]
pub struct FakeLocalCommandRunner {
    commands: RefCell<Vec<LocalCommandInvocation>>,
    responses: RefCell<VecDeque<(String, i32)>>,
}

impl FakeLocalCommandRunner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_responses(responses: Vec<(&str, i32)>) -> Self {
        Self {
            commands: RefCell::new(Vec::new()),
            responses: RefCell::new(
                responses
                    .into_iter()
                    .map(|(output, status)| (output.to_string(), status))
                    .collect(),
            ),
        }
    }

    pub fn commands(&self) -> Vec<LocalCommandInvocation> {
        self.commands.borrow().clone()
    }
}

impl LocalCommandRunner for FakeLocalCommandRunner {
    fn execute(&self, command: &str, args: &[&str]) -> Result<(String, i32)> {
        self.commands.borrow_mut().push(LocalCommandInvocation {
            command: command.to_string(),
            args: args.iter().map(|arg| arg.to_string()).collect(),
        });

        Ok(self
            .responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| (String::new(), 0)))
    }
}

#[derive(Default)]
pub struct FakeRemoteClient {
    commands: RefCell<Vec<String>>,
    streamed_commands: RefCell<Vec<String>>,
    copied_files: RefCell<Vec<(String, String)>>,
    responses: RefCell<VecDeque<(String, i32)>>,
    stream_responses: RefCell<VecDeque<i32>>,
}

impl FakeRemoteClient {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_responses(responses: Vec<(&str, i32)>) -> Self {
        Self {
            responses: RefCell::new(
                responses
                    .into_iter()
                    .map(|(output, status)| (output.to_string(), status))
                    .collect(),
            ),
            ..Default::default()
        }
    }

    pub fn with_stream_responses(stream_responses: Vec<i32>) -> Self {
        Self {
            stream_responses: RefCell::new(stream_responses.into_iter().collect()),
            ..Default::default()
        }
    }

    pub fn commands(&self) -> Vec<String> {
        self.commands.borrow().clone()
    }

    pub fn streamed_commands(&self) -> Vec<String> {
        self.streamed_commands.borrow().clone()
    }

    pub fn copied_files(&self) -> Vec<(String, String)> {
        self.copied_files.borrow().clone()
    }
}

impl RemoteClient for FakeRemoteClient {
    fn execute_command(&self, command: &str) -> Result<(String, i32)> {
        self.commands.borrow_mut().push(command.to_string());
        Ok(self
            .responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| (String::new(), 0)))
    }

    fn execute_command_stream(&self, command: &str) -> Result<i32> {
        self.streamed_commands
            .borrow_mut()
            .push(command.to_string());
        Ok(self.stream_responses.borrow_mut().pop_front().unwrap_or(0))
    }

    fn copy_file(&self, local_path: &str, remote_path: &str) -> Result<()> {
        self.copied_files
            .borrow_mut()
            .push((local_path.to_string(), remote_path.to_string()));
        Ok(())
    }
}
