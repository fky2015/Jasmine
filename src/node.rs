use std::sync::Arc;

use parking_lot::Mutex;
use tokio::task::JoinHandle;
use tracing::info;

use crate::{
    consensus::{Environment, Voter},
    data::{self, Block},
    metrics,
    network::MemoryNetworkAdaptor,
    node_config::NodeConfig,
};

pub(crate) struct Node {
    config: NodeConfig,
    env: Arc<Mutex<Environment>>,
    voter: Voter,
}

impl Node {
    pub(crate) fn new(config: NodeConfig, network: MemoryNetworkAdaptor, genesis: Block) -> Self {
        let block_tree = data::BlockTree::new(genesis, &config);

        let voter_set = config.get_voter_set();
        let state = Arc::new(Mutex::new(Environment::new(block_tree, voter_set, network)));
        let voter = Voter::new(config.get_id(), config.to_owned(), state.to_owned());

        Self {
            env: state,
            voter,
            config,
        }
    }

    pub(crate) fn spawn_run(mut self) -> JoinHandle<()> {
        info!("Node config: {:#?}", self.config);
        println!("Voter is running: {}", self.config.get_id());
        tokio::spawn(async move {
            self.voter.start().await;
        })
    }

    /// Enable and spawn a metrics.
    pub(crate) fn metrics(&self) {
        let (tx, rx) = tokio::sync::mpsc::channel(10000);
        self.env.lock().register_finalized_block_tx(tx);
        self.env.lock().block_tree.enable_metrics();
        let mut metrics = metrics::Metrics::new(self.config.to_owned(), rx);
        tokio::spawn(async move { metrics.dispatch().await });
    }
}
