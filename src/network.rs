use std::{collections::HashMap, sync::Arc};

use crate::{
    consensus::{Message, NetworkPackage},
    Hash,
};
use tokio::sync::mpsc::{self, Receiver, Sender};
// pub trait NetworkDriver<In, Out> {
//     fn handlers(&mut self, view: u32) -> (&mut In, &Out);
// }

// 100 Mb/s
const Throughput: usize = 100;
// ms
const Latency: usize = 1;

pub struct MemoryNetwork {
    sender: HashMap<u64, mpsc::Sender<NetworkPackage>>,
    receiver: mpsc::Receiver<NetworkPackage>,
    network_sender: mpsc::Sender<NetworkPackage>,
}

impl MemoryNetwork {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel(100);
        Self {
            sender: HashMap::new(),
            receiver,
            network_sender: sender,
        }
    }

    pub(crate) fn register(&mut self, id: u64) -> MemoryNetworkAdaptor {
        let (tx, rx) = mpsc::channel(100);
        self.sender.insert(id, tx);
        MemoryNetworkAdaptor {
            receiver: Some(rx),
            network_sender: self.network_sender.clone(),
        }
    }

    pub async fn dispatch(&mut self) {
        while let Some(msg) = self.receiver.recv().await {
            if msg.to.is_none() {
                for sender in self.sender.values() {
                    let msg = msg.clone();
                    sender.send(msg).await;
                }
            } else {
                let to = msg.to.unwrap();
                if let Some(h) = self.sender.get(&to) {
                    h.send(msg).await;
                }
            }
        }
    }
}

pub(crate) struct MemoryNetworkAdaptor {
    receiver: Option<Receiver<NetworkPackage>>,
    network_sender: Sender<NetworkPackage>,
}

impl MemoryNetworkAdaptor {
    pub(crate) fn take_receiver(&mut self) -> Receiver<NetworkPackage> {
        self.receiver.take().unwrap()
    }

    pub(crate) fn get_sender(&self) -> Sender<NetworkPackage> {
        self.network_sender.clone()
    }
}


