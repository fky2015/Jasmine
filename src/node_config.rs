use anyhow::Result;
use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    io::Write,
    net::{SocketAddr, ToSocketAddrs},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    cli::Cli,
    consensus::VoterSet,
    crypto::{generate_keypair, Digest, Keypair, PublicKey, Signature},
};

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
    pub(crate) pretend_failure: bool,
    /// Rotate leadership every `rotate_every` key blocks.
    pub(crate) leader_rotation: usize,
    /// The number of blocks to keep in the ledger.
    pub(crate) gc_depth: usize,
}

impl Default for NodeSettings {
    fn default() -> Self {
        Self {
            transaction_size: 512,
            batch_size: 1000,
            mempool_size: 2000,
            pretend_failure: false,
            leader_rotation: 10000,
            gc_depth: 1000,
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
            injection_rate: 10_000_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum ConsensusType {
    Jasmine {
        /// Consensus Engine will not generate in-between blocks
        /// if actual payload length is less than this value.
        /// This is to prevent the network from being flooded with
        /// empty blocks.
        minimal_batch_size_ratio: f64,
    },
    HotStuff,
}

impl Default for ConsensusType {
    fn default() -> Self {
        Self::Jasmine {
            /// A ratio that determines the minimal batch size.
            /// The minimal batch size is calculated by
            /// `minimal_batch_size = batch_size * minimal_batch_size_ratio`.
            minimal_batch_size_ratio: 0.8,
        }
    }
}

impl std::fmt::Display for ConsensusType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Jasmine {
                minimal_batch_size_ratio: _,
            } => {
                write!(f, "jasmine")
            }
            Self::HotStuff => write!(f, "hotstuff"),
        }
    }
}

impl ConsensusType {
    // pub(crate) fn is_jasmine(&self) -> bool {
    //     matches!(self, Self::Jasmine { .. })
    // }
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub(crate) struct TestMode {
    #[serde(default)]
    pub(crate) delay_test: bool,
    #[serde(default)]
    pub(crate) memory_test: bool,
    #[serde(default)]
    pub(crate) fault_tolerance_test: bool,
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
    /// Stop the node after `n` samples.
    /// If not provided, never stop.
    pub(crate) stop_after_n_samples: Option<usize>,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            enabled: true,
            stop_after: None,
            trace_finalization: false,
            sampling_interval: 500,
            export_path: None,
            stop_after_stable: true,
            stable_threshold: 1.0,
            sampling_window: 20,
            report_every_n_samples: Some(4),
            stop_after_n_samples: Some(30),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct NodeConfig {
    id: PublicKey,
    private_key: Keypair,
    // id, addr
    peer_addrs: BTreeMap<PublicKey, SocketAddr>,

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
        let mut peer_addrs = BTreeMap::new();
        let (id, private_key) = generate_keypair();
        peer_addrs.insert(
            id,
            "localhost:8123".to_socket_addrs().unwrap().next().unwrap(),
        );

        Self {
            id,
            private_key,
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

        config.map(|mut cfg: NodeConfig| {
            if cli.disable_jasmine {
                cfg.consensus = ConsensusType::HotStuff;
            }

            if cfg.metrics.export_path.is_none() {
                cfg.metrics.export_path = cli.export_path.clone();
            }

            if cli.disable_metrics {
                cfg.metrics.enabled = false;
            }

            if cli.pretend_failure {
                cfg.node_settings.pretend_failure = true;
            }

            if let Some(v) = cli.rate {
                cfg.client.injection_rate = v;
            }

            if let Some(v) = cli.transaction_size {
                cfg.node_settings.transaction_size = v;
            }

            if let Some(v) = cli.batch_size {
                cfg.node_settings.batch_size = v;
            }

            if let Some(v) = cli.leader_rotation {
                cfg.node_settings.leader_rotation = v;
            }

            cfg
        })
    }

    pub fn clone_with_keypair(&self, id: PublicKey, private_key: Keypair) -> Self {
        let mut config = self.clone();
        config.id = id;
        config.private_key = private_key;
        config
    }

    pub fn get_id(&self) -> PublicKey {
        self.id
    }

    pub fn get_local_addr(&self) -> Result<&SocketAddr> {
        self.peer_addrs
            .get(&self.id)
            .ok_or_else(|| ConfigError::LocalAddrError.into())
    }

    pub fn get_peer_addrs(&self) -> HashMap<PublicKey, SocketAddr> {
        let mut ret = HashMap::new();
        for (k, v) in &self.peer_addrs {
            ret.insert(*k, *v);
        }
        ret
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
        let v: Vec<_> = self.peer_addrs.keys().cloned().collect();
        VoterSet::new(v)
    }

    pub fn override_voter_set(&mut self, voter_set: &VoterSet) {
        let mut peer_addrs = BTreeMap::new();
        voter_set.iter().for_each(|id| {
            peer_addrs.insert(
                *id,
                "localhost:8123".to_socket_addrs().unwrap().next().unwrap(),
            );
        });

        self.peer_addrs = peer_addrs;
    }

    pub fn get_client_config(&self) -> &ClientConfig {
        &self.client
    }

    pub fn set_peer_addrs(&mut self, peer_addrs: BTreeMap<PublicKey, SocketAddr>) {
        self.peer_addrs = peer_addrs;
    }

    pub fn dry_run(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn export(&self, full_path: &Path) -> Result<()> {
        let mut file = File::create(full_path)?;
        let content = self.dry_run()?;
        file.write_all(content.as_bytes())?;
        Ok(())
    }

    pub fn disable_metrics(&mut self) {
        self.metrics.enabled = false;
    }

    pub fn set_pretend_failure(&mut self) {
        self.node_settings.pretend_failure = true;
    }

    pub fn sign(&self, msg: &Digest) -> Signature {
        self.private_key.sign(msg)
    }

    pub fn get_private_key(&self) -> &Keypair {
        &self.private_key
    }
}
