use std::sync::Arc;

use consensus::{Environment, Message, NetworkPackage, Voter, VoterSet};
use data::Block;
use network::{MemoryNetwork, MemoryNetworkAdaptor};
use parking_lot::Mutex;

mod consensus;
mod data;
mod metrics;
mod network;

pub type Hash = u64;

struct Node {
    env: Arc<Mutex<Environment>>,
    id: u64,
    voter: Voter,
    // network: MemoryNetworkAdaptor,
}

impl Node {
    // TODO: voter_set should read via config
    fn new(id: u64, network: MemoryNetworkAdaptor, genesis: Block, voter_set: VoterSet) -> Self {
        let block_tree = data::BlockTree::new(genesis);

        let state = Arc::new(Mutex::new(Environment::new(block_tree, voter_set, network)));
        let voter = Voter::new(id, state.to_owned());

        Self {
            env: state,
            id,
            voter,
            // network,
        }
    }

    async fn run(&mut self) {
        println!("Voter is running: {}", self.id);
        self.voter.start().await;
    }

    async fn metrics(&self) -> metrics::Metrics {
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        self.env.lock().register_finalized_block_tx(tx);
        self.env.lock().block_tree.enable_metrics();
        metrics::Metrics::new(rx)
    }
}

#[tokio::main]
async fn main() {
    let voter_set: Vec<_> = (0..4).collect();
    let genesis = data::Block::genesis();

    let mut network = MemoryNetwork::new();

    // Prepare the environment.
    let nodes: Vec<_> = voter_set
        .iter()
        .map(|id| {
            let adaptor = network.register(*id);
            Node::new(
                *id,
                adaptor,
                genesis.to_owned(),
                VoterSet::new(voter_set.clone()),
            )
        })
        .collect();

    // Boot up the network.
    let handle = tokio::spawn(async move {
        network.dispatch().await;
    });

    let mut metrics = nodes.get(1).unwrap().metrics().await;

    tokio::spawn(async move {
        metrics.dispatch().await;
    });

    // Run the nodes.
    nodes.into_iter().for_each(|mut node| {
        tokio::spawn(async move {
            node.run().await;
        });
    });

    let _ = tokio::join!(handle);
}
