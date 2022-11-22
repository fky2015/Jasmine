use std::time::Duration;

use tokio::{sync::mpsc::Sender, time::Instant};

use crate::{
    data::{Command, CommandType, Transaction},
    node_config::NodeConfig,
};

pub(crate) struct FakeClient {
    config: NodeConfig,
    transport: Sender<Transaction>,
}

impl FakeClient {
    pub(crate) fn spawn(config: NodeConfig, sender: Sender<Transaction>) {
        let client = FakeClient::new(config, sender);
        tokio::spawn(async move {
            client.send().await;
        });
    }

    fn new(config: NodeConfig, transport: Sender<Transaction>) -> Self {
        Self { config, transport }
    }

    async fn send(&self) {
        /// Amount of txs get send in this
        const PRECISION: u64 = 20;

        // ms
        let interval = 1000f64 / PRECISION as f64;
        let upper_bound = self.config.get_client_config().injection_rate / PRECISION;
        let command_size = self.config.get_node_settings().transaction_size;

        let mut index = 0;

        let mut next_awake = Instant::now() + Duration::from_millis(interval as u64);

        loop {
            for _ in 0..upper_bound {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64;
                let payload = Command {
                    id: index,
                    created_time: now,
                    command_type: CommandType::Set(0, 0),
                };

                let tx = Transaction::from(payload.serialize(command_size - 32));

                self.transport.send(tx.clone()).await.unwrap();
                index += 1;
            }

            tokio::time::sleep_until(next_awake).await;
            next_awake += Duration::from_millis(interval as u64);
        }
    }
}
