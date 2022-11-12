// TODO: use parking_lot::RwLock
// disable unused variable warning
#![allow(unused_variables)]
#![allow(unused_imports)]
#![allow(dead_code)]
use crate::{
    data::{BlockType, QC},
    Hash,
};
use std::{
    collections::HashMap,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    task::Poll,
};

use futures::FutureExt;
use tokio::{
    sync::{
        mpsc::{Receiver, Sender},
        Mutex, Notify,
    },
    task::spawn_local,
};

use crate::{
    data::{Block, BlockTree},
    network::MemoryNetworkAdaptor,
};

#[derive(Debug, Clone)]
pub enum Message {
    Propose(Block),
    ProposeInBetween(Block),
    Vote(Hash),
}

impl Message {}

pub struct VoterState {
    pub id: u64,
    pub view: u64,
    pub threshold: usize,
    pub generic_qc: QC,
    pub locked_qc: QC,
    // <view, (what, whos)>
    pub votes: HashMap<u64, HashMap<u64, Vec<u64>>>,
    pub notify: Arc<Notify>,
}

impl VoterState {
    pub fn new(id: u64, view: u64, generic_qc: QC, threshold: usize) -> Self {
        Self {
            id,
            view,
            threshold,
            generic_qc: generic_qc.to_owned(),
            locked_qc: generic_qc,
            votes: HashMap::new(),
            notify: Arc::new(Notify::new()),
        }
    }
    pub(crate) fn get_notify(&mut self) -> Arc<Notify> {
        self.notify.to_owned()
    }

    pub(crate) fn view_add_one(&mut self) {
        self.view += 1;
        self.notify.notify_waiters();
    }

    pub(crate) fn add_vote(&mut self, msg_view: u64, block_hash: Hash, voter_id: u64) {
        let view_map = self.votes.entry(msg_view).or_default();
        let voters = view_map.entry(block_hash).or_default();
        voters.push(voter_id);

        // TODO: check if we gathered enough votes
        if voters.len() >= self.threshold as usize {}
    }
}

#[derive(Debug, Clone)]
pub struct NetworkPackage {
    pub(crate) id: u64,
    /// None means the message is a broadcast message.
    pub(crate) to: Option<u64>,
    /// None means the message is a global message.
    pub(crate) view: Option<u64>,
    pub(crate) message: Message,
    pub(crate) signature: u64,
}

pub struct Environment {
    block_tree: BlockTree,
    voter_set: VoterSet,
    network: MemoryNetworkAdaptor,
}

impl Environment {
    pub(crate) fn new(
        block_tree: BlockTree,
        voter_set: VoterSet,
        network: MemoryNetworkAdaptor,
    ) -> Self {
        Self {
            block_tree,
            voter_set,
            network,
        }
    }
}

pub(crate) struct Voter {
    id: u64,
    view: u64,
    env: Arc<Mutex<Environment>>,
}

#[derive(Debug, Clone)]
pub(crate) struct VoterSet {
    voters: Vec<u64>,
}

impl VoterSet {
    pub fn new(voters: Vec<u64>) -> Self {
        Self { voters }
    }

    pub fn threshold(&self) -> usize {
        self.voters.len() - (self.voters.len() as f64 / 3.0).ceil() as usize
    }
}

impl Voter {
    pub(crate) fn new(id: u64, env: Arc<Mutex<Environment>>) -> Self {
        let view = 0;
        Self { id, view, env }
    }

    fn package_message(id: u64, message: Message, view: u64, to: Option<u64>) -> NetworkPackage {
        NetworkPackage {
            id,
            to,
            view: Some(view),
            message,
            signature: 0,
        }
    }

    async fn new_in_between_block(
        env: Arc<Mutex<Environment>>,
        view: u64,
        generic_qc: QC,
        id: u64,
    ) -> NetworkPackage {
        let block = env
            .lock()
            .await
            .block_tree
            .new_block(generic_qc, BlockType::InBetween);
        Self::package_message(id, Message::ProposeInBetween(block), view, None)
    }

    async fn new_key_block(
        env: Arc<Mutex<Environment>>,
        view: u64,
        generic_qc: QC,
        id: u64,
    ) -> NetworkPackage {
        let block = env
            .lock()
            .await
            .block_tree
            .new_block(generic_qc, BlockType::Key);
        Self::package_message(id, Message::Propose(block), view, None)
    }

    // -> id
    fn get_leader(view: u64, voters: &VoterSet) -> u64 {
        voters
            .voters
            .get(((view / 100) % voters.voters.len() as u64) as usize)
            .unwrap()
            .to_owned()
    }

    pub(crate) async fn start(&mut self) {
        // Start from view 0, and keep increasing the view number
        let generic_qc = self.env.lock().await.block_tree.genesis().0.justify.clone();
        let voters = self.env.lock().await.voter_set.to_owned();
        let state = Arc::new(Mutex::new(VoterState::new(
            self.id,
            self.view,
            generic_qc,
            voters.threshold(),
        )));
        let notify = Arc::new(AtomicU64::new(0));

        let voter = Self::voter(state.to_owned(), self.env.to_owned(), notify.to_owned());
        let leader = Self::leader(state.to_owned(), self.env.to_owned(), notify.to_owned());

        let handler1 = tokio::spawn(async {
            leader.await;
        });
        let handler2 = tokio::spawn(async {
            voter.await;
        });
        tokio::join!(handler1, handler2);
    }

    async fn voter(
        state: Arc<Mutex<VoterState>>,
        env: Arc<Mutex<Environment>>,
        collect_view: Arc<AtomicU64>,
    ) {
        let id = state.lock().await.id;
        let voters = {
            let env = env.lock().await;
            env.voter_set.to_owned()
        };
        println!("{}: voter get env lock, {:#?}", id, env.try_lock().is_ok());
        println!("{}: voter start", id);

        let (mut rx, tx) = {
            println!("{}: voter get env lock, {:#?}", id, env.try_lock().is_ok());
            let mut env = env.lock().await;
            let rx = env.network.take_receiver();
            let tx = env.network.get_sender();
            (rx, tx)
        };
        println!("{}: voter test", id);
        while let Some(pkg) = rx.recv().await {
            println!("{}: voter: {:?}", id, pkg);
            tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
            let from = pkg.id;
            let view = pkg.view.unwrap();
            let message = pkg.message;
            match message {
                Message::Propose(block) => {
                    // TODO: valid block

                    // TODO: get justify blocks 3-chains
                    let hash = block.hash();

                    env.lock().await.block_tree.add_block(block, BlockType::Key);

                    let current_view = state.lock().await.view;

                    // Suppose the block is valid, vote for it
                    let pkg = Self::package_message(
                        id,
                        Message::Vote(hash),
                        current_view,
                        Some(Self::get_leader(current_view + 1, &voters)),
                    );
                    tx.send(pkg).await.unwrap();

                    // Finish the view
                    state.lock().await.view_add_one();
                }
                Message::ProposeInBetween(block) => {
                    // TODO: valid block
                    let hash = block.hash();

                    // Add block to block tree
                    env.lock().await.block_tree.add_block(block, BlockType::Key);
                }
                Message::Vote(block_hash) => {
                    state.lock().await.add_vote(view, block_hash, from);
                }
            }
            // TODO: if it froms current view, then process it
            // TODO: if it froms previous view, then ignore it
            // TODO: if it froms future view, then buffer it

            // TODO: if it is a propose, save it to block_tree
            // TODO: if it is a propose_key_block, then process it, and let view = view + 1
        }
    }

    async fn leader(
        state: Arc<Mutex<VoterState>>,
        env: Arc<Mutex<Environment>>,
        collect_view: Arc<AtomicU64>,
    ) {
        let id = state.lock().await.id;
        println!("{}: leader start", id);

        loop {
            let tx = env.lock().await.network.get_sender();
            println!("{}: leader: test", id);
            // TODO: no need to clone.
            let voters = env.lock().await.voter_set.clone();
            let view = state.lock().await.view;
            if Self::get_leader(view, &voters) == id {
                println!("{}: leader: I am the leader of view {}", id, view);
                /// qc for in-between blocks
                let generic_qc = state.lock().await.generic_qc.clone();

                let timeout = tokio::time::sleep(tokio::time::Duration::from_millis(1000));
                tokio::pin!(timeout);

                let fu = futures::future::poll_fn(|cx| Poll::Ready(()));

                // TODO: loop until timeout or get enough votes
                while collect_view.load(Ordering::SeqCst) != view {
                    println!("{}: leader: view: {}", id, view);
                    let pkg =
                        Self::new_in_between_block(env.to_owned(), view, generic_qc.to_owned(), id)
                            .await;
                    tx.send(pkg).await.unwrap();
                }

                let generic_qc = state.lock().await.generic_qc.clone();
                let pkg = Self::new_key_block(env.to_owned(), view, generic_qc, id).await;
                tx.send(pkg).await.unwrap();
            }

            let notify = state.lock().await.notify.clone();
            // awake if the view is changed.
            notify.notified().await;
        }
    }
}
