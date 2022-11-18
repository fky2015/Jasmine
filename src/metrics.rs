use std::{
    process::exit,
    time::{SystemTime, UNIX_EPOCH},
};

use tokio::sync::mpsc::Receiver;

use crate::{
    data::{Block, BlockType},
    node_config::NodeConfig,
};

pub(crate) struct Metrics {
    config: NodeConfig,
    finalized_block_rx: Receiver<(Block, BlockType, u64)>,
    total_delay: u64,
    finalized_transactions: u64,
    finalized_blocks: u64,
    start_time: u64,
    type_counts: [u64; 2],
    key_delay: u64,
    key_totals: u64,
}

impl Metrics {
    pub fn new(config: NodeConfig, finalized_block_rx: Receiver<(Block, BlockType, u64)>) -> Self {
        Self {
            config,
            finalized_block_rx,
            total_delay: 0,
            key_delay: 0,
            finalized_transactions: 0,
            start_time: 0,
            type_counts: [0, 0],
            key_totals: 0,
            finalized_blocks: 0,
        }
    }

    pub async fn dispatch(&mut self) {
        let metrics_config = self.config.get_metrics();
        while let Some((block, block_type, finalized_time)) = self.finalized_block_rx.recv().await {
            self.finalized_blocks += 1;
            self.total_delay += (finalized_time - block.timestamp) * block.payloads.len() as u64;
            self.finalized_transactions += block.payloads.len() as u64;
            let current_time = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;

            if self.start_time == 0 || self.start_time > block.timestamp {
                self.start_time = block.timestamp;
            }

            if block_type == BlockType::Key {
                self.type_counts[0] += 1;
                self.key_totals += block.payloads.len() as u64;
                self.key_delay += (finalized_time - block.timestamp) * block.payloads.len() as u64;
            } else {
                self.type_counts[1] += 1;
            }

            if metrics_config.trace_finalization {
                tracing::trace!(
                    "Finalized block {} with {} transactions in {} ms, start: {}",
                    block.height,
                    block.payloads.len(),
                    finalized_time - block.timestamp,
                    block.timestamp
                );
            }

            // TODO: 计算 critcal path 的吞吐率
            if let Some(period) = metrics_config.period {
                if self.finalized_blocks % period == 0 {
                    tracing::info!(
                    "[Metrics] Average delay: {} sec, Finalized transaction: {}, Throughput: {} Kops/sec, key:in-between: {}:{}, finalized_blocks: {}, key_delay: {}",
                    self.total_delay as f64 / self.finalized_transactions as f64 / 1000.0,
                    self.finalized_transactions,
                    self.finalized_transactions as f64/ (current_time - self.start_time) as f64,
                    self.type_counts[0],
                    self.type_counts[1],
                    self.finalized_blocks,
                    self.key_delay as f64 / self.key_totals as f64 / 1000.0
                );
                }
            }

            if let Some(height) = metrics_config.stop_after {
                if self.finalized_blocks > height {
                    // TODO: Export data to file first.
                    tracing::info!("Node exits");
                    exit(0);
                }
            }
        }
    }
}
