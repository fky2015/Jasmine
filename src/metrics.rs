use std::{
    collections::VecDeque,
    process::exit,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
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
    start_time: Instant,
    type_counts: [u64; 2],
    key_delay: u64,
    key_totals: u64,
    sample_window: VecDeque<MetricsSample>,
    next_sample_index: usize,
}

impl Metrics {
    pub fn new(config: NodeConfig, finalized_block_rx: Receiver<(Block, BlockType, u64)>) -> Self {
        let next_sample_index = 1;

        Self {
            config,
            finalized_block_rx,
            total_delay: 0,
            key_delay: 0,
            finalized_transactions: 0,
            start_time: Instant::now(),
            type_counts: [0, 0],
            key_totals: 0,
            finalized_blocks: 0,
            sample_window: VecDeque::new(),
            next_sample_index,
        }
    }

    pub async fn dispatch(&mut self) {
        while let Some((block, block_type, finalized_time)) = self.finalized_block_rx.recv().await {
            // blocks statistics
            self.finalized_blocks += 1;
            self.finalized_transactions += block.payloads.len() as u64;

            if block_type == BlockType::Key {
                self.type_counts[0] += 1;
                self.key_totals += block.payloads.len() as u64;
                self.key_delay += (finalized_time - block.timestamp) * block.payloads.len() as u64;
            } else {
                self.type_counts[1] += 1;
            }

            // transaction statistics

            self.total_delay += (finalized_time - block.timestamp) * block.payloads.len() as u64;

            if self.config.get_metrics().trace_finalization {
                tracing::trace!(
                    "Finalized block {} with {} transactions in {} ms, start: {}",
                    block.height,
                    block.payloads.len(),
                    finalized_time - block.timestamp,
                    block.timestamp
                );
            }

            // TODO: 计算 critcal path 的吞吐率
            if self.start_time.elapsed()
                > (Duration::from_millis(self.config.get_metrics().sampling_interval)
                    * self.next_sample_index as u32)
            {
                self.sample();

                self.next_sample_index += 1;
            }

            self.try_exit();
        }
    }

    fn sample(&mut self) {
        let sample = MetricsSample::from_metrics(self);
        if let Some(report_interval) = self.config.get_metrics().report_every_n_samples {
            if self.next_sample_index % report_interval == 0 {
                tracing::info!("{}", sample);
            }
        }
        self.sample_window.push_front(sample);
        if self.sample_window.len() > self.config.get_metrics().sampling_window {
            self.sample_window.pop_back();
        }
    }

    /// Sign of exits
    fn high_enough(&self) -> bool {
        if let Some(height) = self.config.get_metrics().stop_after {
            self.finalized_blocks > height
        } else {
            false
        }
    }

    /// Sign of exits
    fn stable(&self) -> bool {
        if self.sample_window.len() < self.config.get_metrics().sampling_window {
            return false;
        }

        std_deviation(
            &self
                .sample_window
                .iter()
                .map(|d| d.average_delay)
                .collect::<Vec<_>>(),
        )
        .unwrap()
            < self.config.get_metrics().stable_threshold
            && std_deviation(
                &self
                    .sample_window
                    .iter()
                    .map(|d| d.consensus_throughput)
                    .collect::<Vec<_>>(),
            )
            .unwrap()
                < self.config.get_metrics().stable_threshold
    }

    fn try_exit(&mut self) {
        if self.high_enough() || self.stable() {
            // TODO: export data to file first.
            tracing::info!("Node exits");
            exit(0);
        }
    }
}

// TODO: Every batch size?
struct MetricsSample {
    // ms
    average_delay: f64,
    finalized_transactions: u64,
    consensus_throughput: f64,
    key_block_ratio: f64,
    finalized_blocks: u64,
    // ms
    key_block_delay: f64,
    average_batch_size: f64,
}

impl MetricsSample {
    fn from_metrics(m: &Metrics) -> Self {
        let average_delay = m.total_delay as f64 / m.finalized_transactions as f64;
        let finalized_transactions = m.finalized_transactions;
        let consensus_throughput =
            m.finalized_transactions as f64 / m.start_time.elapsed().as_millis() as f64;

        let key_block_ratio = m.type_counts[0] as f64 / m.finalized_blocks as f64;
        let finalized_blocks = m.finalized_blocks;
        let key_block_delay = m.key_delay as f64 / m.key_totals as f64;

        let average_batch_size = m.finalized_transactions as f64 / m.finalized_blocks as f64;
        Self {
            average_delay,
            finalized_transactions,
            consensus_throughput,
            key_block_ratio,
            finalized_blocks,
            key_block_delay,
            average_batch_size,
        }
    }
}

impl std::fmt::Display for MetricsSample {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Average delay: {} ms, Throughput: {} Kops/sec, key:in-between: {}:{}, finalized_blocks: {}, average_batch_size: {}, key_delay: {}",
            self.average_delay,
            self.consensus_throughput,
            self.key_block_ratio,
            1.0 - self.key_block_ratio,
            self.finalized_blocks,
            self.average_batch_size,
            self.key_block_delay
        )
    }
}

fn mean(data: &[f64]) -> Option<f64> {
    let sum: f64 = data.iter().sum();
    let count = data.len();

    match count {
        positive if positive > 0 => Some(sum / count as f64),
        _ => None,
    }
}

fn std_deviation(data: &[f64]) -> Option<f64> {
    match (mean(data), data.len()) {
        (Some(data_mean), count) if count > 0 => {
            let variance = data
                .iter()
                .map(|value| {
                    let diff = data_mean - *value;

                    diff * diff
                })
                .sum::<f64>()
                / count as f64;

            Some(variance.sqrt())
        }
        _ => None,
    }
}
