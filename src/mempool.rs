use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::mpsc::Sender;

use crate::{
    data::{CommandGetter, Transaction},
    node_config::NodeConfig,
};

pub(crate) struct Mempool {
    config: NodeConfig,
    pool: Arc<Mutex<Vec<Transaction>>>,
}

impl Mempool {
    pub fn spawn_receiver(config: NodeConfig) -> (Self, Sender<Transaction>) {
        let (tx, mut rx) = tokio::sync::mpsc::channel(10);
        let mempool_size = config.get_node_settings().mempool_size;
        let mempool = Mempool::new(config);
        let pool = mempool.pool.clone();
        tokio::spawn(async move {
            while let Some(tx) = rx.recv().await {
                loop {
                    if pool.lock().len() < mempool_size {
                        pool.lock().push(tx);
                        break;
                    }

                    tokio::task::yield_now().await;
                }
            }
        });
        (mempool, tx)
    }

    fn new(config: NodeConfig) -> Self {
        Self {
            config,
            pool: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl CommandGetter for Mempool {
    fn get_commands(&mut self) -> Vec<Transaction> {
        {
            let mut pool = self.pool.lock();

            if pool.len() >= self.config.get_node_settings().batch_size {
                pool.drain(..self.config.get_node_settings().batch_size)
                    .collect()
            } else {
                pool.drain(..).collect()
            }
        }
    }

    fn get_commands_with_lowerbound(&mut self, minimal: usize) -> Option<Vec<Transaction>> {
        {
            let mut pool = self.pool.lock();

            if pool.len() >= minimal {
                if pool.len() >= self.config.get_node_settings().batch_size {
                    Some(
                        pool.drain(..self.config.get_node_settings().batch_size)
                            .collect(),
                    )
                } else {
                    Some(pool.drain(..).collect())
                }
            } else {
                None
            }
        }
    }
}
