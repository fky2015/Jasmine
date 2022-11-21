#![feature(drain_filter)]
// TODO: metrics critical path to see what affects performance.

use std::{
    collections::{BTreeMap, HashMap},
    env,
    fs::{self, File},
    io::Write,
    net::ToSocketAddrs,
    path::{Path, PathBuf},
};

use clap::Parser;
use cli::{Cli, Commands};
use consensus::VoterSet;
use network::{MemoryNetwork, TcpNetwork};
use node::Node;

use anyhow::Result;
use node_config::NodeConfig;
use serde::{Serialize, Serializer};
use tracing::field::display;

mod cli;
mod client;
mod consensus;
mod data;
mod mempool;
mod metrics;
mod network;
mod node;
mod node_config;

pub type Hash = u64;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut config = crate::node_config::NodeConfig::from_cli(&cli)?;

    tracing_subscriber::fmt::init();

    match cli.command {
        Some(Commands::MemoryTest { number }) => {
            let voter_set: Vec<_> = (0..number).collect();
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
            let total = number * 3 + 1;
            let mut voter_set: Vec<_> = (0..total).collect();
            let genesis = data::Block::genesis();
            let mut network = MemoryNetwork::new();

            // Mock peers
            config.override_voter_set(&VoterSet::new(voter_set.to_owned()));

            voter_set.retain(|id| !(1..=number).contains(id));

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
        Some(Commands::ConfigGen {
            number,
            mut hosts,
            export_dir,
            write_file,
        }) => {
            // println!("Generating config {:?}", cfg);
            if hosts.is_empty() {
                println!("No hosts provided, use localhost instead.");
                hosts.push(String::from("localhost"))
            }

            let distribution_plan = DistributionPlan::new(number, hosts, config);

            if !write_file {
                for (path, content) in distribution_plan.dry_run(&export_dir)? {
                    println!("{}", path.display());
                    println!("{}", content);
                }
            } else if !Path::new(&export_dir).is_dir() {
                panic!("Export dir is not a directory");
            } else {
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

struct ConfigFile {
    config: NodeConfig,
}

impl ConfigFile {
    fn new(config: NodeConfig) -> Self {
        Self { config }
    }

    fn dry_run(&self) -> Result<(PathBuf, String)> {
        self.config.dry_run().map(|content| {
            let mut path = PathBuf::new();
            path.push(format!("{}.json", self.config.get_id()));
            (path, content)
        })
    }

    fn export(&self, path: &Path) -> Result<()> {
        self.config.export(path)
    }
}

struct ExecutionPlan {
    // Vec<(path to config file)>
    configs: Vec<ConfigFile>,
    host: String,
}

impl ExecutionPlan {
    fn new(host: String) -> Self {
        Self {
            configs: Vec::new(),
            host,
        }
    }

    fn add(&mut self, config: NodeConfig) {
        self.configs.push(ConfigFile::new(config));
    }

    /// Generate execution plan script to run the nodes.
    fn dry_run(&self) -> Result<Vec<(PathBuf, String)>> {
        let parent_dir = Path::new(&self.host);
        let mut ret = Vec::new();
        for config in self.configs.iter() {
            let pair = config.dry_run()?;
            ret.push(pair);
        }

        let mut content: String = "#!/bin/bash\n".to_string();
        for pair in ret.iter() {
            content.push_str(&format!("./jasmine --config {} &\n", pair.0.display()));
        }
        let path = Path::new("run.sh");
        ret.push((path.to_path_buf(), content));

        let ret = ret
            .into_iter()
            .map(|(path, content)| {
                let path = parent_dir.join(path);
                (path, content)
            })
            .collect();

        Ok(ret)
    }
}

struct DistributionPlan {
    execution_plans: Vec<ExecutionPlan>,
}

/// Generate a distribution plan to distribute corresponding files.
///
/// `scp -r $BASE_DIR/$HOST-n $HOST_N`
impl DistributionPlan {
    fn new(number: u64, hosts: Vec<String>, base_config: NodeConfig) -> Self {
        let voter_set: Vec<_> = (0..number).collect();

        let mut peer_addrs = HashMap::new();

        voter_set.iter().for_each(|&id| {
            peer_addrs.insert(
                id,
                format!(
                    "{}:{}",
                    hosts.get(id as usize % hosts.len()).unwrap().to_owned(),
                    8123 + id
                )
                .to_socket_addrs()
                .unwrap()
                .next()
                .unwrap(),
            );
        });

        let mut execution_plans = Vec::new();
        for host in &hosts {
            execution_plans.push(ExecutionPlan::new(host.to_owned()));
        }

        voter_set.into_iter().for_each(|id| {
            let mut config = base_config.clone_with_id(id);
            config.set_peer_addrs(peer_addrs.clone());

            let index = id as usize % hosts.len();

            execution_plans
                .get_mut(index)
                .expect("hosts length is the same as execution_plans")
                .add(config);
        });

        DistributionPlan { execution_plans }
    }

    fn dry_run(&self, parent_dir: &Path) -> Result<Vec<(PathBuf, String)>> {
        let mut ret = Vec::new();

        self.execution_plans.iter().for_each(|plan| {
            let plan = plan.dry_run().unwrap();
            ret.extend_from_slice(plan.as_slice());
        });

        let mut release_binary = env::current_dir()?;
        release_binary.push("target");
        release_binary.push("release");
        release_binary.push("jasmine");

        if !release_binary.exists() {
            panic!("please build jasmine release first!");
        }

        let mut content = "#!/bin/bash\ncargo build --release\n".to_string();
        self.execution_plans
            .iter()
            .enumerate()
            .for_each(|(_i, plan)| {
                content.push_str(&format!("scp -r {}/* {}:\n", &plan.host, plan.host));
                content.push_str(&format!(
                    "scp -r {} {}:\n",
                    release_binary.display(),
                    plan.host
                ));
            });

        ret.push((Path::new("distribute.sh").to_path_buf(), content));

        let ret = ret
            .into_iter()
            .map(|(path, content)| {
                let path = parent_dir.join(path);
                (path, content)
            })
            .collect();

        Ok(ret)
    }
}

//test
#[cfg(test)]
mod test {}
