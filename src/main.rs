#![feature(drain_filter)]
// TODO: metrics critical path to see what affects performance.
use std::{sync::Arc, thread, time::Duration};

use crate::node_config::{Cli, Commands, NodeConfig};
use clap::Parser;
use consensus::{Environment, Voter, VoterSet};
use data::Block;
use network::{MemoryNetwork, MemoryNetworkAdaptor, TcpNetwork};
use parking_lot::{Mutex, deadlock};
use tokio::task::JoinHandle;
use tracing::info;


use anyhow::Result;

mod client;
mod consensus;
mod data;
mod mempool;
mod metrics;
mod network;
mod node_config;

pub type Hash = u64;

struct Node {
    config: NodeConfig,
    env: Arc<Mutex<Environment>>,
    voter: Voter,
}

impl Node {
    fn new(config: NodeConfig, network: MemoryNetworkAdaptor, genesis: Block) -> Self {
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

    fn spawn_run(mut self) -> JoinHandle<()> {
        info!("Node config: {:#?}", self.config);
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
        let mut metrics = metrics::Metrics::new(self.config.to_owned(), rx);
        tokio::spawn(async move { metrics.dispatch().await });
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut config = crate::node_config::NodeConfig::from_cli(&cli)?;

    tracing_subscriber::fmt::init();

    // Create a background thread which checks for deadlocks every 10s
    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(5));
        let deadlocks = deadlock::check_deadlock();
        if deadlocks.is_empty() {
            continue;
        }

        println!("{} deadlocks detected", deadlocks.len());
        for (i, threads) in deadlocks.iter().enumerate() {
            println!("Deadlock #{}", i);
            for t in threads {
                println!("Thread Id {:#?}", t.thread_id());
                println!("{:#?}", t.backtrace());
            }
        }
        panic!("Deadlock detected");
    });

    match &cli.command {
        Some(Commands::MemoryTest { number }) => {
            let voter_set: Vec<_> = (0..*number).collect();
            let genesis = data::Block::genesis();

            let mut network = MemoryNetwork::new();

            // Mock peers
            config.override_voter_set(&VoterSet::new(voter_set.to_owned()));

            // Prepare the environment.
            let nodes: Vec<_> = voter_set
                .iter()
                .map(|id| {
                    let adaptor = network.register(*id);
                    Node::new(config.clone_with_id(*id), adaptor, genesis.to_owned())
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
        Some(Commands::FailTest { number }) => {
            let total = *number * 3 + 1;
            let mut voter_set: Vec<_> = (0..total).collect();
            let genesis = data::Block::genesis();
            let mut network = MemoryNetwork::new();

            // Mock peers
            config.override_voter_set(&VoterSet::new(voter_set.to_owned()));

            voter_set.retain(|id| !(1..=*number).contains(id));

            // Prepare the environment.
            let nodes: Vec<_> = voter_set
                .iter()
                .map(|id| {
                    let adaptor = network.register(*id);
                    Node::new(config.clone_with_id(*id), adaptor, genesis.to_owned())
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
            let adapter = TcpNetwork::spawn(
                config.get_local_addr()?.to_owned(),
                config.get_peer_addrs().to_owned(),
            );

            let node = Node::new(config, adapter, data::Block::genesis());

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
