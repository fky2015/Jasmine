use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use std::{collections::HashMap, net::SocketAddr, time::Duration};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use tracing::{error, info, warn};

use crate::{consensus::NetworkPackage, crypto::PublicKey};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::mpsc::{self, channel, Receiver, Sender},
};

use anyhow::Result;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum NetworkError {
    #[error("Failed to recv message: {0}")]
    Recv(#[from] std::io::Error),
    #[error("Failed to deserialize message: {0}")]
    Deserialize(#[from] Box<bincode::ErrorKind>),
    #[error("Failed to config TCP_NODELAY to true")]
    TcpNoDelay(std::io::Error),
    #[error("Faild to send message: {0} to {1}")]
    Send(String, String),
    #[error("Failed to send to mpsc: {0}")]
    MpscSend(#[from] Box<mpsc::error::SendError<NetworkPackage>>),
}

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

    pub async fn dispatch(&mut self) -> Result<()> {
        while let Some(msg) = self.receiver.recv().await {
            if msg.to.is_none() {
                for sender in self.sender.values() {
                    let msg = msg.clone();
                    sender
                        .send(msg)
                        .await
                        .map_err(|e| NetworkError::Send(e.to_string(), "All".to_string()))?;
                }
            } else {
                let to = msg.to.as_ref().expect("to must be Some(_)");
                if let Some(h) = self.sender.get(to) {
                    h.send(msg).await?;
                }
            }
        }

        Ok(())
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
    sender_rx: Receiver<NetworkPackage>,
}

impl FailureNetwork {
    /// Create a new `FailureNetwork`.
    ///
    /// * `addr`: the local address to listen on.
    /// * `_config`:
    pub fn spawn(mut addr: SocketAddr, _config: HashMap<PublicKey, SocketAddr>) -> NetworkAdaptor {
        let (tx, rx) = mpsc::channel(1);
        let (sender_tx, sender_rx) = mpsc::channel(1);
        if !addr.ip().is_loopback() {
            // Set self bind address to 0.0.0.0.
            addr.set_ip("0.0.0.0".parse().unwrap());
        }
        tokio::spawn(async move { Self { sender_rx }.run().await });
        FailureReceiver::spawn(addr, tx);

        NetworkAdaptor {
            network_sender: sender_tx,
            receiver: Some(rx),
        }
    }

    async fn run(&mut self) {
        while let Some(_pkg) = self.sender_rx.recv().await {}
    }
}

pub struct TcpNetwork {
    // this belongs to the config object.
    addrs: HashMap<PublicKey, SocketAddr>,
    sender: SimpleSender,
    sender_rx: Receiver<NetworkPackage>,
}

impl TcpNetwork {
    pub fn spawn(
        mut addr: SocketAddr,
        mut config: HashMap<PublicKey, SocketAddr>,
    ) -> NetworkAdaptor {
        let (tx, rx) = mpsc::channel(1);
        let (sender_tx, sender_rx) = mpsc::channel(1);
        if !addr.ip().is_loopback() {
            // Set self address to 0.0.0.0.
            for v in config.values_mut() {
                if v.ip() == addr.ip() {
                    info!("set same hosts {} address to device lo", v);
                    v.set_ip("127.0.0.1".parse().unwrap());
                }
            }
            // Set self bind address to 0.0.0.0.
            addr.set_ip("0.0.0.0".parse().unwrap());
        }

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

    async fn run(&mut self) -> Result<()> {
        while let Some(pkg) = self.sender_rx.recv().await {
            let to = pkg.to;
            let pkg = bincode::serialize(&pkg).unwrap();
            if to.is_none() {
                for addr in self.addrs.values() {
                    self.sender.send(*addr, pkg.clone().into()).await?;
                }
            } else if let Some(addr) = self.addrs.get(&to.unwrap()) {
                self.sender.send(*addr, pkg.into()).await?;
            }
        }
        Ok(())
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
    pub async fn send(&mut self, address: SocketAddr, data: Bytes) -> Result<()> {
        let connection = self
            .connections
            .entry(address)
            .or_insert_with(|| Self::spawn_connection(address));
        connection.send(data).await?;

        Ok(())
    }
}

struct Connection {
    address: SocketAddr,
    receiver: Receiver<Bytes>,
}

impl Connection {
    fn spawn(address: SocketAddr, receiver: Receiver<Bytes>) {
        tokio::spawn(async move {
            Self { address, receiver }.run().await?;

            Ok::<(), anyhow::Error>(())
        });
    }

    async fn run(&mut self) -> Result<()> {
        // Try to connect to the peer
        let (mut writer, mut reader) = loop {
            match TcpStream::connect(self.address).await {
                Ok(stream) => {
                    // Turn on TCP_NODELAY to eliminate latency.
                    stream.set_nodelay(true).map_err(NetworkError::TcpNoDelay)?;
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
                    writer.send(bytes).await?;
                }


                Some(result) = reader.next() => {
                    result.map_err(NetworkError::Recv)?;
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

    async fn spawn_runner(socket: TcpStream, _peer: SocketAddr, _sender: Sender<NetworkPackage>) {
        tokio::spawn(async move {
            let (mut _writer, mut reader) =
                Framed::new(socket, LengthDelimitedCodec::new()).split();
            while let Some(_result) = reader.next().await {}
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
        let listener = loop {
            match TcpListener::bind(&self.address).await {
                Ok(listener) => break listener,
                Err(err) => {
                    warn!("Failed to bind address: {}", err);
                }
            }

            tokio::time::sleep(Duration::from_millis(500)).await;
        };

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

    async fn spawn_runner(socket: TcpStream, _peer: SocketAddr, sender: Sender<NetworkPackage>) {
        tokio::spawn(async move {
            let (mut _writer, mut reader) =
                Framed::new(socket, LengthDelimitedCodec::new()).split();
            while let Some(result) = reader.next().await {
                let bytes = result.map_err(NetworkError::Recv)?;
                let msg = bincode::deserialize::<NetworkPackage>(&bytes)
                    .map_err(NetworkError::Deserialize)?;
                sender.send(msg).await?;
            }

            Ok::<(), anyhow::Error>(())
        });
    }
}
