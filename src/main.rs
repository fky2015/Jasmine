#![feature(drain_filter)]
// TODO: metrics critical path to see what affects performance.

use crate::config_gen::DistributionPlan;
use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

use clap::Parser;
use cli::{Cli, Commands};
use consensus::VoterSet;
use crypto::generate_keypairs;
use network::{FailureNetwork, MemoryNetwork, TcpNetwork};
use node::Node;

use anyhow::Result;

mod cli;
mod client;
mod config_gen;
mod consensus;
mod crypto;
mod data;
mod mempool;
mod metrics;
mod network;
mod node;
mod node_config;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut config = crate::node_config::NodeConfig::from_cli(&cli)?;

    tracing_subscriber::fmt::init();

    match cli.command {
        Some(Commands::MemoryTest { number }) => {
            let voter_set: Vec<_> = generate_keypairs(number);
            let genesis = data::Block::genesis();

            let mut network = MemoryNetwork::new();

            // Mock peers
            config.override_voter_set(&VoterSet::new(
                voter_set.iter().map(|(pk, _)| *pk).collect(),
            ));

            // Prepare the environment.
            let nodes: Vec<_> = voter_set
                .into_iter()
                .map(|(id, secret)| {
                    let adaptor = network.register(id);
                    Node::new(
                        config.clone_with_keypair(id, secret),
                        adaptor,
                        genesis.to_owned(),
                    )
                })
                .collect();

            // Boot up the network.
            let handle = tokio::spawn(async move {
                network.dispatch().await?;
                Ok::<_, anyhow::Error>(())
            });

            nodes.get(0).unwrap().metrics();

            // Run the nodes.
            nodes.into_iter().for_each(|node| {
                node.spawn_run();
            });

            let _ = tokio::join!(handle);
        }
        Some(Commands::FailTest { number }) => {
            let total = number * 3 + 1;
            let voter_set: Vec<_> = generate_keypairs(total);
            let genesis = data::Block::genesis();
            let mut network = MemoryNetwork::new();

            // Mock peers
            config.override_voter_set(&VoterSet::new(
                voter_set.iter().map(|(pk, _)| *pk).collect(),
            ));

            // Prepare the environment.
            let nodes: Vec<_> = voter_set
                .iter()
                .enumerate()
                .filter_map(|(idx, (p, sec))| {
                    if idx % 3 == 1 {
                        // Fail the node.
                        None
                    } else {
                        let adaptor = network.register(*p);
                        Some(Node::new(
                            config.clone_with_keypair(*p, sec.clone()),
                            adaptor,
                            genesis.to_owned(),
                        ))
                    }
                })
                .collect();

            // Boot up the network.
            let handle = tokio::spawn(async move {
                network.dispatch().await?;
                Ok::<_, anyhow::Error>(())
            });

            nodes.get(0).unwrap().metrics();

            // Run the nodes.
            nodes.into_iter().for_each(|node| {
                node.spawn_run();
            });

            let _ = tokio::join!(handle);
        }
        Some(Commands::ConfigGen {
            number,
            mut hosts,
            mut export_dir,
            write_file,
            failure_nodes,
            auto_naming,
        }) => {
            if !auto_naming && export_dir.is_none() {
                panic!("export_dir must be specified when auto_naming is false");
            } else if auto_naming && export_dir.is_none() {
                let mut i = 0;
                while Path::new(&format!("config_{}", i)).exists() {
                    i += 1;
                }

                let name = format!("config_{}", i);

                export_dir = Some(PathBuf::from(name));
            }

            let export_dir = export_dir.expect("export_dir must be specified");

            // println!("Generating config {:?}", cfg);
            if hosts.is_empty() {
                println!("No hosts provided, use localhost instead.");
                hosts.push(String::from("localhost"))
            }

            let distribution_plan = DistributionPlan::new(number, hosts, config, failure_nodes);

            if !write_file {
                for (path, content) in distribution_plan.dry_run(&export_dir)? {
                    println!("{}", path.display());
                    println!("{}", content);
                }
            } else {
                if !Path::new(&export_dir).is_dir() {
                    fs::create_dir_all(&export_dir)?;
                }

                for (path, content) in distribution_plan.dry_run(&export_dir)? {
                    let dir = path.parent().unwrap();
                    if !dir.exists() {
                        fs::create_dir(dir)?;
                    }
                    let mut file = File::create(path)?;
                    file.write_all(content.as_bytes())?;
                }
            }
        }
        None => {
            let adapter = if config.get_node_settings().pretend_failure {
                FailureNetwork::spawn(config.get_local_addr()?.to_owned(), config.get_peer_addrs())
            } else {
                TcpNetwork::spawn(config.get_local_addr()?.to_owned(), config.get_peer_addrs())
            };

            tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

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
