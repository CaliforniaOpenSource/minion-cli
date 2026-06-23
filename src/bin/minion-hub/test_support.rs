//! Shared fakes and fixtures for hub unit tests.

use anyhow::{anyhow, Result};
use std::cell::RefCell;
use std::collections::VecDeque;

use crate::command::CommandRunner;
use crate::model::{save_hosts, save_wireguard_config, WgConfig};
use crate::paths::HubPaths;
use crate::TEST_PRIVATE_KEY as ROOT_TEST_PRIVATE_KEY;

pub(crate) const TEST_PRIVATE_KEY: &str = ROOT_TEST_PRIVATE_KEY;
pub(crate) const PEER_KEY: &str = "e5ryaK4/d3GriUqOgaGnNRWYglvSXXmLwSmSsTvPZDk=";
pub(crate) const PEER_KEY_2: &str = "n3ST37Z5J9dm00wbUPn2mfNOGIo3iHnMKmtdSJuJONE=";

#[derive(Default)]
pub(crate) struct NoopRunner {
    commands: RefCell<Vec<String>>,
}

impl CommandRunner for NoopRunner {
    fn enabled(&self) -> bool {
        false
    }

    fn run(&self, program: &str, args: &[&str]) -> Result<String> {
        self.commands
            .borrow_mut()
            .push(format!("{} {}", program, args.join(" ")));
        Ok(String::new())
    }

    fn run_with_stdin(&self, program: &str, args: &[&str], _stdin: &str) -> Result<String> {
        self.run(program, args)
    }
}

pub(crate) struct ScriptedRunner {
    responses: RefCell<VecDeque<Result<String, String>>>,
    commands: RefCell<Vec<Vec<String>>>,
}

impl ScriptedRunner {
    pub(crate) fn new(responses: Vec<Result<&str, &str>>) -> Self {
        Self {
            responses: RefCell::new(
                responses
                    .into_iter()
                    .map(|response| {
                        response
                            .map(|value| value.to_string())
                            .map_err(|value| value.to_string())
                    })
                    .collect(),
            ),
            commands: RefCell::new(Vec::new()),
        }
    }

    pub(crate) fn commands(&self) -> Vec<Vec<String>> {
        self.commands.borrow().clone()
    }
}

impl CommandRunner for ScriptedRunner {
    fn enabled(&self) -> bool {
        true
    }

    fn run(&self, program: &str, args: &[&str]) -> Result<String> {
        let mut command = vec![program.to_string()];
        command.extend(args.iter().map(|arg| arg.to_string()));
        self.commands.borrow_mut().push(command);

        match self
            .responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| Ok(String::new()))
        {
            Ok(output) => Ok(output),
            Err(error) => Err(anyhow!(error)),
        }
    }

    fn run_with_stdin(&self, program: &str, args: &[&str], _stdin: &str) -> Result<String> {
        self.run(program, args)
    }
}

pub(crate) fn prepared_store() -> (tempfile::TempDir, HubPaths, NoopRunner) {
    let dir = tempfile::tempdir().unwrap();
    let paths = HubPaths::under_root(dir.path());
    let config = WgConfig {
        private_key: TEST_PRIVATE_KEY.to_string(),
        peers: Vec::new(),
    };
    save_wireguard_config(&paths, &config).unwrap();
    save_hosts(&paths, &[]).unwrap();
    (dir, paths, NoopRunner::default())
}
