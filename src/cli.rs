use std::path::PathBuf;

use clap::Parser;

#[derive(Parser)]
#[command(name = "Jasmine")]
#[command(author = "Feng Kaiyu <loveress01@outlook.com>")]
#[command(about = "POC Jasmine", version)]
pub(crate) struct Cli {
    /// Path to the config file
    #[arg(short, long, value_name = "FILE")]
    pub(crate) config: Option<PathBuf>,

    /// Set ID of current node.
    #[arg(short, long, default_value_t = 0)]
    pub(crate) id: u64,

    /// Addresses of known nodes.
    #[arg(short, long, default_value_t = String::from("localhost:8123"))]
    addr: String,

    /// Use HotStuff consensus.
    #[arg(short, long)]
    pub(crate) disable_jasmine: bool,

    /// Enable delay_test.
    #[arg(long)]
    enable_delay_test: bool,

    /// Pretend to be a failure nodes.
    #[arg(long)]
    pub(crate) pretend_failure: bool,

    /// Disable metrics
    #[arg(long)]
    pub(crate) disable_metrics: bool,

    /// Export metrics to file
    #[arg(short, long)]
    pub(crate) export_path: Option<PathBuf>,

    #[command(subcommand)]
    pub(crate) command: Option<Commands>,
}

#[derive(Parser, Debug)]
pub(crate) enum Commands {
    /// Run the node within memory network.
    MemoryTest {
        /// Number of nodes.
        #[arg(short, long, default_value_t = 4)]
        number: usize,
    },

    FailTest {
        /// Number of failures.
        #[arg(short, long, default_value_t = 1)]
        number: usize,
    },

    /// Generate all configs for a single test.
    ///
    /// This command distributes `number` replicas into `hosts`,
    /// automatically assign id and addr.
    ConfigGen {
        /// Number of nodes.
        #[arg(short, long, default_value_t = 4)]
        number: usize,

        /// hosts to distribute replicas.
        hosts: Vec<String>,

        /// Path to export configs.
        #[arg(short, long, value_name = "DIR")]
        export_dir: PathBuf,

        /// Write all the configs and scripts to file.
        #[arg(short, long)]
        write_file: bool,

        /// Number of failure nodes
        ///
        /// This value must follow the rule of BFT,
        /// that 3f+1 <= n.
        #[arg(short, long, default_value_t = 0)]
        failure_nodes: usize,
    },
}
