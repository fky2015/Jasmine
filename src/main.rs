// TODO: metrics critical path to see what affects performance.
use std::{
    collections::HashMap,
    net::{SocketAddr, SocketAddrV4, ToSocketAddrs},
    path::PathBuf,
    sync::Arc,
};

use crate::config::Config;
use clap::{Parser, Subcommand};
use consensus::{Environment, Message, NetworkPackage, Voter, VoterSet};
use data::Block;
use network::{MemoryNetwork, MemoryNetworkAdaptor, TcpNetwork};
use parking_lot::Mutex;
use tokio::task::JoinHandle;
use tracing_subscriber::FmtSubscriber;

use anyhow::Result;

mod config;
mod consensus;
mod data;
mod metrics;
mod network;

pub type Hash = u64;

struct Node {
    config: Config,
    env: Arc<Mutex<Environment>>,
    voter: Voter,
}

impl Node {
    // TODO: voter_set should read via config
    fn new(
        config: Config,
        network: MemoryNetworkAdaptor,
        genesis: Block,
        voter_set: VoterSet,
    ) -> Self {
        let block_tree = data::BlockTree::new(genesis);

        let state = Arc::new(Mutex::new(Environment::new(block_tree, voter_set, network)));
        let voter = Voter::new(config.get_id(), config.to_owned(), state.to_owned());

        Self {
            env: state,
            voter,
            config,
        }
    }

    fn spawn_run(mut self) -> JoinHandle<()> {
        println!("Voter is running: {}", self.config.get_id());
        tokio::spawn(async move {
            self.voter.start().await;
        })
    }

    /// Enable and spawn a metrics.
    fn metrics(&self) {
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        self.env.lock().register_finalized_block_tx(tx);
        self.env.lock().block_tree.enable_metrics();
        let mut metrics = metrics::Metrics::new(rx);
        tokio::spawn(async move { metrics.dispatch().await });
    }
}

#[derive(Parser)]
#[command(author, about, version)]
struct Cli {
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    #[arg(short, long)]
    id: Option<u64>,

    #[arg(short, long)]
    addr: Option<String>,

    #[arg(short, long)]
    disable_jasmine: bool,

    #[arg(long)]
    enable_delay_test: bool,

    #[arg(long)]
    disable_metrics: bool,

    #[arg(short, long)]
    sleep_until: Option<String>,

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

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    FmtSubscriber::builder()
        .with_max_level(tracing::Level::INFO)
        .init();

    tracing::info!("Starting node");

    match &cli.command {
        Some(Commands::MemoryTest { number }) => {
            let config = crate::config::Config::from_cli(&cli)?;
            let voter_set: Vec<_> = (0..*number).collect();
            let genesis = data::Block::genesis();

            let mut network = MemoryNetwork::new();

            // Prepare the environment.
            let nodes: Vec<_> = voter_set
                .iter()
                .map(|id| {
                    let adaptor = network.register(*id);
                    Node::new(
                        config.to_owned(),
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

            nodes.get(0).unwrap().metrics();

            // Run the nodes.
            nodes.into_iter().for_each(|node| {
                node.spawn_run();
            });

            let _ = tokio::join!(handle);
        }
        None => {
            let config = crate::config::Config::from_cli(&cli)?;

            let adapter = TcpNetwork::spawn(
                config.get_local_addr()?.to_owned(),
                config.get_peer_addrs().to_owned(),
            );

            let node = Node::new(
                config,
                adapter,
                data::Block::genesis(),
                VoterSet::new(vec![0]),
            );

            if !cli.disable_metrics {
                node.metrics();
            }

            // Run the node
            let handle = node.spawn_run();

            let _ = handle.await;
        }
    }
    Ok(())
}

//test
#[cfg(test)]
mod test {}
