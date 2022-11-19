use anyhow::Result;
use clap::Parser;
use std::{
    collections::HashMap,
    net::{SocketAddr, ToSocketAddrs},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::consensus::VoterSet;

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
    /// The maximum number of transactions in the mempool.
    ///
    /// For best performance, this should be a multiple of batch_size.
    pub(crate) mempool_size: usize,
}

impl Default for NodeSettings {
    fn default() -> Self {
        Self {
            transaction_size: 256,
            batch_size: 100,
            mempool_size: 1000,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub(crate) struct ClientConfig {
    /// Use a instant command generator instead of clients and mempools.
    ///
    /// In this way, end-to-end latency **cannot** be measured.
    pub(crate) use_instant_generator: bool,
    /// Transaction per second.
    pub(crate) injection_rate: u64,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            use_instant_generator: false,
            injection_rate: 1_000_000,
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
    /// Report metrics every `sampling_interval` ms.
    pub(crate) sampling_interval: u64,
    /// Export the metrics data to the `export_path` before the node exits.
    pub(crate) export_path: Option<PathBuf>,
    /// Track last `n` sampling data.
    pub(crate) sampling_window: usize,
    /// Stop the node after mean latency and throughput are stable.
    pub(crate) stop_after_stable: bool,
    /// Stable threshold for mean latency and throughput.
    pub(crate) stable_threshold: f64,
    /// Print the metrics data to stdout every n samples.
    /// If not provided, never report.
    pub(crate) report_every_n_samples: Option<usize>,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            enabled: true,
            stop_after: None,
            trace_finalization: false,
            sampling_interval: 2000,
            export_path: None,
            stop_after_stable: true,
            stable_threshold: 0.1,
            sampling_window: 20,
            report_every_n_samples: Some(1),
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
    #[serde(default)]
    client: ClientConfig,
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
            client: ClientConfig::default(),
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

    pub fn get_voter_set(&self) -> VoterSet {
        VoterSet::new(self.peer_addrs.keys().cloned().collect())
    }

    pub fn get_client_config(&self) -> &ClientConfig {
        &self.client
    }
}

#[derive(Parser)]
#[command(name = "Jasmine")]
#[command(author = "Feng Kaiyu <loveress01@outlook.com>")]
#[command(about = "POC Jasmine", version)]
pub(crate) struct Cli {
    /// Path to the config file
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Set ID of current node.
    #[arg(short, long, default_value_t = 0)]
    id: u64,

    /// Addresses of known nodes.
    #[arg(short, long, default_value_t = String::from("localhost:8123"))]
    addr: String,

    /// Use HotStuff consensus.
    #[arg(short, long)]
    pub(crate) disable_jasmine: bool,

    /// Enable delay_test.
    #[arg(long)]
    enable_delay_test: bool,

    /// Disable metrics
    #[arg(long)]
    pub(crate) disable_metrics: bool,

    #[command(subcommand)]
    pub(crate) command: Option<Commands>,
}

#[derive(Parser)]
pub(crate) enum Commands {
    /// Run the node within memory network.
    MemoryTest {
        /// Number of nodes.
        #[arg(short, long, default_value_t = 4)]
        number: u64,
    },
}
