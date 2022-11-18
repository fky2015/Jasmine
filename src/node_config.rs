use anyhow::Result;
use clap::Parser;
use std::{
    collections::HashMap,
    net::{SocketAddr, ToSocketAddrs},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Failed to parse config file: {0}")]
    ParseError(#[from] config::ConfigError),

    #[error("Wrong local_addr")]
    LocalAddrError,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub(crate) struct NodeSettings {
    pub(crate) transaction_size: usize,
    pub(crate) batch_size: usize,
}

impl Default for NodeSettings {
    fn default() -> Self {
        Self {
            transaction_size: 256,
            batch_size: 100,
        }
    }
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
    /// Enable metrics module.
    enabled: bool,
    /// Stop the node if finalized block is higher than this value.
    /// If not set, the node will run forever.
    pub(crate) stop_after: Option<u64>,
    /// Print every finalization logs.
    pub(crate) trace_finalization: bool,
    /// Report metrics every `period`.
    /// If not provided, never report.
    pub(crate) period: Option<u64>,
    /// Export the metrics data to the `export_path` before the node exits.
    pub(crate) export_path: Option<PathBuf>,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            enabled: true,
            stop_after: Some(1000000),
            trace_finalization: false,
            period: Some(200),
            export_path: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct NodeConfig {
    id: u64,
    // id, addr
    peer_addrs: HashMap<u64, SocketAddr>,

    #[serde(default)]
    node_settings: NodeSettings,
    #[serde(default)]
    consensus: ConsensusType,
    #[serde(default)]
    test_mode: TestMode,
    #[serde(default)]
    logs: Logs,
    #[serde(default)]
    metrics: Metrics,
}

impl Default for NodeConfig {
    fn default() -> Self {
        let mut peer_addrs = HashMap::new();
        peer_addrs.insert(
            0,
            "localhost:8123".to_socket_addrs().unwrap().next().unwrap(),
        );

        Self {
            id: 0,
            peer_addrs,
            node_settings: NodeSettings::default(),
            consensus: ConsensusType::default(),
            test_mode: TestMode::default(),
            logs: Logs::default(),
            metrics: Metrics::default(),
        }
    }
}

impl NodeConfig {
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
        config.map(|mut cfg: NodeConfig| {
            if cli.disable_jasmine {
                cfg.consensus = ConsensusType::HotStuff;
            }

            cfg
        })
    }

    pub fn clone_with_id(&self, id: u64) -> Self {
        let mut config = self.clone();
        config.id = id;
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

    pub fn get_node_settings(&self) -> &NodeSettings {
        &self.node_settings
    }

    pub fn get_metrics(&self) -> &Metrics {
        &self.metrics
    }
}

#[derive(Parser)]
#[command(author, about, version)]
pub(crate) struct Cli {
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    #[arg(short, long)]
    id: Option<u64>,

    #[arg(short, long)]
    addr: Option<String>,

    #[arg(short, long)]
    pub(crate) disable_jasmine: bool,

    #[arg(long)]
    enable_delay_test: bool,

    #[arg(long)]
    pub(crate) disable_metrics: bool,

    #[arg(short, long)]
    sleep_until: Option<String>,

    #[command(subcommand)]
    pub(crate) command: Option<Commands>,
}

#[derive(Parser)]
pub(crate) enum Commands {
    /// Run the node with memory network.
    MemoryTest {
        #[arg(short, long, default_value_t = 4)]
        number: u64,
    },
}
