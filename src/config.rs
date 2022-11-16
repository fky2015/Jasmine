use anyhow::Result;
use std::{
    collections::HashMap,
    net::{SocketAddr, ToSocketAddrs},
};

use crate::Cli;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Failed to parse config file: {0}")]
    ParseError(#[from] config::ConfigError),

    #[error("Wrong local_addr")]
    LocalAddrError,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum ConsensusType {
    Jasmine,
    HotStuff,
}

impl Default for ConsensusType {
    fn default() -> Self {
        Self::Jasmine
    }
}

impl ConsensusType {
    pub(crate) fn is_jasmine(&self) -> bool {
        matches!(self, Self::Jasmine)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub(crate) struct TestMode {
    #[serde(default)]
    pub(crate) delay_test: bool,
    #[serde(default)]
    pub(crate) memory_test: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub(crate) struct Logs {}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct Metrics {
    enabled: bool,
}

impl Default for Metrics {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Config {
    id: u64,
    // id, addr
    peer_addrs: HashMap<u64, SocketAddr>,

    #[serde(default)]
    consensus: ConsensusType,
    #[serde(default)]
    test_mode: TestMode,
    #[serde(default)]
    logs: Logs,
    #[serde(default)]
    metrics: Metrics,
}

impl Default for Config {
    fn default() -> Self {
        let mut peer_addrs = HashMap::new();
        peer_addrs.insert(
            0,
            "localhost:8123".to_socket_addrs().unwrap().next().unwrap(),
        );

        Self {
            id: 0,
            peer_addrs,
            consensus: ConsensusType::default(),
            test_mode: TestMode::default(),
            logs: Logs::default(),
            metrics: Metrics::default(),
        }
    }
}

impl Config {
    pub fn from_cli(cli: &Cli) -> Result<Self, ConfigError> {
        let config = match &cli.config {
            Some(path) => {
                let settings = config::Config::builder()
                    .add_source(config::File::with_name(path.to_str().unwrap()))
                    .add_source(config::Environment::with_prefix("JASMINE"))
                    .build()
                    .unwrap();

                Ok(settings.try_deserialize()?)
            }
            None => Ok(Default::default()),
        };

        // TODO: Override config with cli

        config
    }

    pub fn get_id(&self) -> u64 {
        self.id
    }

    pub fn get_local_addr(&self) -> Result<&SocketAddr> {
        self.peer_addrs
            .get(&self.id)
            .ok_or_else(|| ConfigError::LocalAddrError.into())
    }

    pub fn get_peer_addrs(&self) -> &HashMap<u64, SocketAddr> {
        &self.peer_addrs
    }

    pub fn get_consensus_type(&self) -> &ConsensusType {
        &self.consensus
    }

    pub fn get_test_mode(&self) -> &TestMode {
        &self.test_mode
    }
}
