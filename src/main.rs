use std::{
    collections::HashMap,
    net::{SocketAddr, SocketAddrV4},
    path::PathBuf,
    sync::Arc,
};

use clap::{Parser, Subcommand};
use consensus::{Environment, Message, NetworkPackage, Voter, VoterSet};
use data::Block;
use network::{MemoryNetwork, MemoryNetworkAdaptor, TcpNetwork};
use parking_lot::Mutex;
use tracing_subscriber::FmtSubscriber;

mod consensus;
mod data;
mod metrics;
mod network;

pub type Hash = u64;
pub const DELAY_TEST: bool = true;

struct Node {
    env: Arc<Mutex<Environment>>,
    id: u64,
    voter: Voter,
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

#[derive(Parser)]
#[command(author, about, version)]
struct Cli {
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Parser)]
enum Commands {
    /// Run the node with memory network.
    MemoryTest {
        #[arg(short, long, default_value_t = 4)]
        number: u64,
    },
}

struct Config {}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match &cli.command {
        Some(Commands::MemoryTest { number }) => {
            let voter_set: Vec<_> = (0..*number).collect();
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
        None => {
            let _my_subscriber = FmtSubscriber::builder()
                .with_max_level(tracing::Level::TRACE)
                .init();

            tracing::info!("Starting node");

            // TODO: construct config objects
            let addr: SocketAddr =
                std::net::SocketAddr::V4(SocketAddrV4::new([127, 0, 0, 1].into(), 8080));
            let mut addrs = HashMap::new();
            addrs.insert(0, addr);

            let adapter = TcpNetwork::spawn(addr, addrs);

            let mut node = Node::new(0, adapter, data::Block::genesis(), VoterSet::new(vec![0]));

            let mut metrics = node.metrics().await;

            tokio::spawn(async move {
                metrics.dispatch().await;
            });

            // Run the node
            let handle = tokio::spawn(async move {
                node.run().await;
            });

            let _ = tokio::join!(handle);
        }
    }
}

//test
#[cfg(test)]
mod test {}
