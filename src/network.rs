// TODO: use thiserror
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use std::{collections::HashMap, net::SocketAddr, time::Duration};
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use crate::{consensus::NetworkPackage, crypto::PublicKey};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::mpsc::{self, channel, Receiver, Sender},
};
// pub trait NetworkDriver<In, Out> {
//     fn handlers(&mut self, view: u32) -> (&mut In, &Out);
// }

pub struct MemoryNetwork {
    sender: HashMap<PublicKey, mpsc::Sender<NetworkPackage>>,
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

    pub(crate) fn register(&mut self, id: PublicKey) -> MemoryNetworkAdaptor {
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
                let to = msg.to.as_ref().unwrap();
                if let Some(h) = self.sender.get(&to) {
                    h.send(msg).await;
                }
            }
        }
    }
}

pub struct MemoryNetworkAdaptor {
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

pub type NetworkAdaptor = MemoryNetworkAdaptor;

/// Like `TcpNetwork`, but always drop packages.
pub struct FailureNetwork {
    addrs: HashMap<PublicKey, SocketAddr>,
    sender_rx: Receiver<NetworkPackage>,
}

impl FailureNetwork {
    pub fn spawn(addr: SocketAddr, config: HashMap<PublicKey, SocketAddr>) -> NetworkAdaptor {
        let (tx, rx) = mpsc::channel(1);
        let (sender_tx, sender_rx) = mpsc::channel(1);
        tokio::spawn(async move {
            Self {
                addrs: config.clone(),
                sender_rx,
            }
            .run()
            .await
        });
        FailureReceiver::spawn(addr, tx);

        NetworkAdaptor {
            network_sender: sender_tx,
            receiver: Some(rx),
        }
    }

    async fn run(&mut self) {
        while let Some(_pkg) = self.sender_rx.recv().await {
            // let to = pkg.to;
            // let pkg = bincode::serialize(&pkg).unwrap();
            // if to.is_none() {
            //     for addr in self.addrs.values() {
            //         self.sender.send(*addr, pkg.clone().into()).await;
            //     }
            // } else if let some(addr) = self.addrs.get(&to.unwrap()) {
            //     self.sender.send(*addr, pkg.into()).await;
            // }
        }
    }
}

pub struct TcpNetwork {
    // this belongs to the config object.
    addrs: HashMap<PublicKey, SocketAddr>,
    sender: SimpleSender,
    sender_rx: Receiver<NetworkPackage>,
}

impl TcpNetwork {
    pub fn spawn(addr: SocketAddr, config: HashMap<PublicKey, SocketAddr>) -> NetworkAdaptor {
        let (tx, rx) = mpsc::channel(1);
        let (sender_tx, sender_rx) = mpsc::channel(1);
        tokio::spawn(async move {
            Self {
                addrs: config.clone(),
                sender: SimpleSender::new(),
                sender_rx,
            }
            .run()
            .await
        });
        SimpleReceiver::spawn(addr, tx);

        NetworkAdaptor {
            network_sender: sender_tx,
            receiver: Some(rx),
        }
    }

    async fn run(&mut self) {
        while let Some(pkg) = self.sender_rx.recv().await {
            tracing::trace!("sending {:?}", pkg);
            let to = pkg.to.clone();
            let pkg = bincode::serialize(&pkg).unwrap();
            if to.is_none() {
                for addr in self.addrs.values() {
                    self.sender.send(*addr, pkg.clone().into()).await;
                }
            } else if let Some(addr) = self.addrs.get(&to.unwrap()) {
                self.sender.send(*addr, pkg.into()).await;
            }
        }
    }
}

pub struct SimpleSender {
    /// A map holding the channels to our connections.
    connections: HashMap<SocketAddr, Sender<bytes::Bytes>>,
}

impl std::default::Default for SimpleSender {
    fn default() -> Self {
        Self::new()
    }
}

impl SimpleSender {
    pub fn new() -> Self {
        Self {
            connections: HashMap::new(),
        }
    }

    fn spawn_connection(address: SocketAddr) -> Sender<Bytes> {
        let (tx, rx) = channel(1);
        Connection::spawn(address, rx);
        tx
    }

    /// Try to send a message to a peer.
    pub async fn send(&mut self, address: SocketAddr, data: Bytes) {
        let connection = self
            .connections
            .entry(address)
            .or_insert_with(|| Self::spawn_connection(address));
        connection.send(data).await.unwrap();
    }

    // Try to broadcast a message to all peers.
    // pub async fn broadcast(&mut self, addresses: Vec<SocketAddr>, data: Bytes) {
    //     for address in addresses {
    //         self.send(address, data.clone()).await;
    //     }
    // }
}

struct Connection {
    address: SocketAddr,
    receiver: Receiver<Bytes>,
}

impl Connection {
    fn spawn(address: SocketAddr, receiver: Receiver<Bytes>) {
        tokio::spawn(async move {
            Self { address, receiver }.run().await;
        });
    }

    async fn run(&mut self) {
        // Try to connect to the peer
        let (mut writer, mut reader) = loop {
            match TcpStream::connect(self.address).await {
                Ok(stream) => {
                    // Turn on TCP_NODELAY to eliminate latency.
                    stream.set_nodelay(true).unwrap();
                    break Framed::new(stream, LengthDelimitedCodec::new()).split();
                }
                Err(e) => {
                    eprintln!("failed to connect to {}: {}", self.address, e);
                    tokio::time::sleep(Duration::from_millis(800)).await;
                }
            };
        };

        loop {
            tokio::select! {
                Some(bytes) = self.receiver.recv() => {
                    if let Err(e) = writer.send(bytes).await {
                        eprintln!("failed to send message to {}: {}", self.address, e);
                        return;
                    }

                    tracing::trace!("sent to {}", self.address);
                }


                Some(result) = reader.next() => {
                    match result {
                        Ok(_bytes) => {
                            unreachable!()
                            // let msg = Message::decode(&bytes);
                            // println!("Received message from {}: {:?}", self.address, msg);
                        }
                        Err(e) => {
                            eprintln!("failed to receive message from {}: {}", self.address, e);
                            return;
                        }
                    }
                }
            }
        }
    }
}

pub struct FailureReceiver {
    /// Local address to listen on.
    address: SocketAddr,
    /// A tx channel to send received messages to.
    sender: Sender<NetworkPackage>,
}

impl FailureReceiver {
    pub fn spawn(address: SocketAddr, sender: Sender<NetworkPackage>) {
        tokio::spawn(async move {
            Self { address, sender }.run().await;
        });
    }
    async fn run(&self) {
        let listener = TcpListener::bind(&self.address)
            .await
            .expect("Failed to bind address");

        loop {
            let (socket, peer) = match listener.accept().await {
                Ok(value) => value,
                Err(e) => {
                    eprintln!("failed to accept socket; error = {:?}", e);
                    continue;
                }
            };

            Self::spawn_runner(socket, peer, self.sender.clone()).await;
        }
    }

    async fn spawn_runner(socket: TcpStream, peer: SocketAddr, sender: Sender<NetworkPackage>) {
        tokio::spawn(async move {
            let (mut _writer, mut reader) =
                Framed::new(socket, LengthDelimitedCodec::new()).split();
            while let Some(result) = reader.next().await {}
        });
    }
}

pub struct SimpleReceiver {
    /// Local address to listen on.
    address: SocketAddr,
    /// A tx channel to send received messages to.
    sender: Sender<NetworkPackage>,
}

impl SimpleReceiver {
    pub fn spawn(address: SocketAddr, sender: Sender<NetworkPackage>) {
        tokio::spawn(async move {
            Self { address, sender }.run().await;
        });
    }
    async fn run(&self) {
        let listener = TcpListener::bind(&self.address)
            .await
            .expect("Failed to bind address");

        loop {
            let (socket, peer) = match listener.accept().await {
                Ok(value) => value,
                Err(e) => {
                    eprintln!("failed to accept socket; error = {:?}", e);
                    continue;
                }
            };

            Self::spawn_runner(socket, peer, self.sender.clone()).await;
        }
    }

    async fn spawn_runner(socket: TcpStream, peer: SocketAddr, sender: Sender<NetworkPackage>) {
        tokio::spawn(async move {
            let (mut _writer, mut reader) =
                Framed::new(socket, LengthDelimitedCodec::new()).split();
            while let Some(result) = reader.next().await {
                match result {
                    Ok(bytes) => match bincode::deserialize::<NetworkPackage>(&bytes) {
                        Ok(msg) => {
                            sender.send(msg).await.unwrap();
                        }
                        Err(e) => {
                            eprintln!("failed to deserialize message from {}: {}", peer, e);
                        }
                    },
                    Err(e) => {
                        eprintln!("failed to receive message from {}: {}", peer, e);
                        return;
                    }
                }
            }
        });
    }
}
