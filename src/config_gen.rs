use std::{
    collections::BTreeMap,
    env,
    net::ToSocketAddrs,
    path::{Path, PathBuf},
};

use crate::{crypto::generate_keypairs, node_config::NodeConfig};
use anyhow::Result;

/// Represent a single config file.
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
}

/// Represent a bundle of config files which are executed in a single server.
struct ExecutionPlan {
    // Vec<(path to config file)>
    configs: Vec<ConfigFile>,
    host: String,
    /// Start from 0
    index: usize,
}

impl ExecutionPlan {
    fn new(index: usize, host: String) -> Self {
        Self {
            configs: Vec::new(),
            host,
            index,
        }
    }

    fn add(&mut self, config: NodeConfig) {
        self.configs.push(ConfigFile::new(config));
    }

    /// Generate execution plan script to run the nodes.
    fn dry_run(&self, result_label: &str) -> Result<Vec<(PathBuf, String)>> {
        let parent_dir = Path::new(&self.host);
        let mut ret = Vec::new();
        for config in self.configs.iter() {
            let pair = config.dry_run()?;
            ret.push(pair);
        }

        let mut content: String = "#!/bin/bash\n".to_string();
        let mut trap_threads_line = "trap 'kill".to_string();
        for (i, pair) in ret.iter().enumerate() {
            if i == 0 && self.index == 0 {
                content.push_str(&format!(
                    "./jasmine --config {} --export-path result-{}.json &\n",
                    pair.0.display(),
                    result_label
                ));
            } else {
                content.push_str(&format!(
                    "./jasmine --config {} --disable-metrics &>/dev/null &\n",
                    pair.0.display()
                ));
            }
            content.push_str(&format!("THREAD_{}=$!\n", i));
            trap_threads_line.push_str(&format!(" $THREAD_{}", i));
        }
        trap_threads_line.push_str("' SIGINT SIGTERM\n");
        content.push_str(&trap_threads_line);
        content.push_str("wait $THREAD_0\n");
        content.push_str("sleep 2\n");
        content.push_str("pkill -P $$\n");

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

pub(crate) struct DistributionPlan {
    execution_plans: Vec<ExecutionPlan>,
    result_label: String,
}

/// Generate a distribution plan to distribute corresponding files.
///
/// `scp -r $BASE_DIR/$HOST-n $HOST_N`
impl DistributionPlan {
    pub(crate) fn new(
        number: usize,
        hosts: Vec<String>,
        base_config: NodeConfig,
        failure_nodes: usize,
    ) -> Self {
        let voter_set: Vec<_> = generate_keypairs(number);

        let mut peer_addrs = BTreeMap::new();

        voter_set
            .iter()
            .enumerate()
            .for_each(|(idx, (pub_key, _priv_key))| {
                peer_addrs.insert(
                    *pub_key,
                    format!(
                        "{}:{}",
                        hosts.get(idx % hosts.len()).unwrap().to_owned(),
                        8123 + idx
                    )
                    .to_socket_addrs()
                    .unwrap()
                    .next()
                    .unwrap(),
                );
            });

        let mut execution_plans = Vec::new();
        for (idx, host) in hosts.iter().enumerate() {
            execution_plans.push(ExecutionPlan::new(idx, host.to_owned()));
        }

        voter_set
            .into_iter()
            .enumerate()
            .for_each(|(idx, (pub_key, priv_key))| {
                let mut config = base_config.clone_with_keypair(pub_key, priv_key);
                config.set_peer_addrs(peer_addrs.clone());

                // The last `failure_nodes` nodes will be marked as failure nodes.
                if idx >= number - failure_nodes {
                    config.set_pretend_failure();
                }

                let index = idx % hosts.len();

                execution_plans
                    .get_mut(index)
                    .expect("hosts length is the same as execution_plans")
                    .add(config);
            });

        DistributionPlan {
            execution_plans,
            result_label: Self::get_result_label(number, hosts.len(), failure_nodes, base_config),
        }
    }

    fn get_result_label(
        number: usize,
        hosts_len: usize,
        failure_nodes: usize,
        base_config: NodeConfig,
    ) -> String {
        format!(
            "{}-n{}-h{}-f{}-ts{}-bs{}-ir{}-lr{}",
            base_config.get_consensus_type(),
            number,
            hosts_len,
            failure_nodes,
            base_config.get_node_settings().transaction_size,
            base_config.get_node_settings().batch_size,
            base_config.get_client_config().injection_rate,
            base_config.get_node_settings().leader_rotation,
        )
    }

    fn new_run_scripts(&self, hosts: &Vec<String>) -> Result<Vec<(PathBuf, String)>> {
        let mut ret = Vec::new();
        let mut content: String = "#!/bin/bash\n".to_string();
        for host in hosts {
            content.push_str(&format!("ssh {} \"bash run.sh\" &\n", host));
        }

        ret.push((Path::new("run-remotes.sh").to_path_buf(), content));

        let mut content: String = "#!/bin/bash\n".to_string();

        let first_host = hosts.get(0).expect("Hosts cannot be empty.");

        content.push_str(&format!(
            "while ! scp {}:result-{}.json result-{}.json 2>/dev/null; do sleep 5; done\n",
            first_host, self.result_label, self.result_label
        ));

        ret.push((Path::new("get-results.sh").to_path_buf(), content));

        let mut content: String = "#!/bin/bash\nset -x\n".to_string();
        content.push_str("bash kill-all.sh\n");
        content.push_str("bash distribute.sh\n");
        content.push_str("bash run-remotes.sh\n");
        content.push_str("bash get-results.sh\n");
        content.push_str("bash clean-all.sh\n");

        ret.push((Path::new("run-all.sh").to_path_buf(), content));

        let mut content: String = "#!/bin/bash\n".to_string();
        for host in hosts {
            content.push_str(&format!("ssh {} 'rm ./*.json' &\n", host));
            content.push_str(&format!("ssh {} 'killall jasmine' &\n", host));
            content.push_str(&format!("rm -r {}\n", host));
        }

        content.push_str("rm run-all.sh\n");

        ret.push((Path::new("clean-all.sh").to_path_buf(), content));

        let mut content: String = "#!/bin/bash\n".to_string();
        for host in hosts {
            content.push_str(&format!("ssh {} 'rm ./*.json' &\n", host));
            content.push_str(&format!("ssh {} 'killall jasmine' &\n", host));
        }
        ret.push((Path::new("kill-all.sh").to_path_buf(), content));

        Ok(ret)
    }

    pub(crate) fn dry_run(&self, parent_dir: &Path) -> Result<Vec<(PathBuf, String)>> {
        let mut ret = Vec::new();

        self.execution_plans.iter().for_each(|plan| {
            let plan = plan.dry_run(&self.result_label).unwrap();
            ret.extend_from_slice(plan.as_slice());
        });

        let mut copy_release = "#!/bin/bash\ncargo build --release\n".to_string();
        let mut release_binary = env::current_dir()?;
        release_binary.push("target");
        release_binary.push("release");
        release_binary.push("jasmine");

        let mut content = "#!/bin/bash\nset -ex\n".to_string();
        self.execution_plans
            .iter()
            .enumerate()
            .for_each(|(_i, plan)| {
                content.push_str(&format!("scp -r {}/* {}:\n", &plan.host, plan.host));
                copy_release.push_str(&format!(
                    "scp -r {} {}:\n",
                    release_binary.display(),
                    plan.host
                ));
            });

        ret.push((Path::new("distribute.sh").to_path_buf(), content));

        ret.push((Path::new("copy-release.sh").to_path_buf(), copy_release));

        ret.extend_from_slice(
            self.new_run_scripts(
                &self
                    .execution_plans
                    .iter()
                    .map(|plan| plan.host.to_owned())
                    .collect::<Vec<_>>(),
            )
            .unwrap()
            .as_slice(),
        );

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
