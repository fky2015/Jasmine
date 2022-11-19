use crate::{
    data::{BlockType, QC},
    node_config::NodeConfig,
    Hash,
};
use std::{
    collections::HashMap,
    process::exit,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use tokio::sync::{mpsc::Sender, Notify};

use serde::{Deserialize, Serialize};

use parking_lot::Mutex;

use crate::{
    data::{Block, BlockTree},
    network::MemoryNetworkAdaptor,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub best_view: Arc<AtomicU64>,
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
            best_view: Arc::new(AtomicU64::new(0)),
        }
    }
    pub(crate) fn get_notify(&mut self) -> Arc<Notify> {
        self.notify.to_owned()
    }

    pub(crate) fn view_add_one(&mut self) {
        // println!("{}: view add to {}", self.id, self.view + 1);
        self.votes.retain(|v, _| v >= &self.view);
        self.view += 1;
        self.notify.notify_waiters();
    }

    pub(crate) fn add_vote(&mut self, msg_view: u64, block_hash: Hash, voter_id: u64) {
        // println!("add_vote: {:?} {:?} {:?}", msg_view, block_hash, voter_id);
        let view_map = self.votes.entry(msg_view).or_default();
        let voters = view_map.entry(block_hash).or_default();
        // TODO: check if voter_id is already in voters
        voters.push(voter_id);

        if voters.len() >= self.threshold {
            // println!("{}: best_view update to {}", self.id, msg_view);
            self.best_view.store(msg_view, Ordering::Relaxed);
            // TODO: partial signature
            self.generic_qc = QC {
                node: block_hash,
                view: msg_view,
            };
        }
    }

    pub(crate) fn best_view_ref(&self) -> Arc<AtomicU64> {
        self.best_view.to_owned()
    }

    pub(crate) fn get_best_view(&self) -> u64 {
        self.best_view.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
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
    pub(crate) block_tree: BlockTree,
    voter_set: VoterSet,
    network: MemoryNetworkAdaptor,
    pub(crate) finalized_block_tx: Option<Sender<(Block, BlockType, u64)>>,
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
            finalized_block_tx: None,
        }
    }

    pub(crate) fn register_finalized_block_tx(&mut self, tx: Sender<(Block, BlockType, u64)>) {
        self.finalized_block_tx = Some(tx);
    }
}

pub(crate) struct Voter {
    id: u64,
    config: NodeConfig,
    /// Only used when initialize ConsensusVoter.
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
    pub(crate) fn new(id: u64, config: NodeConfig, env: Arc<Mutex<Environment>>) -> Self {
        let view = 1;
        Self {
            id,
            config,
            view,
            env,
        }
    }

    pub(crate) async fn start(&mut self) {
        // Start from view 0, and keep increasing the view number
        let generic_qc = self.env.lock().block_tree.genesis().0.justify.clone();
        let voters = self.env.lock().voter_set.to_owned();
        let state = Arc::new(Mutex::new(VoterState::new(
            self.id,
            self.view,
            generic_qc,
            voters.threshold(),
        )));
        let notify = state.lock().best_view_ref();

        let voter = ConsensusVoter::new(
            self.config.to_owned(),
            state.to_owned(),
            self.env.to_owned(),
            notify.to_owned(),
        );
        let leader = voter.clone();

        let handler1 = tokio::spawn(async {
            leader.run_as_leader().await;
        });
        let handler2 = tokio::spawn(async {
            voter.run_as_voter().await;
        });
        tokio::join!(handler1, handler2);
    }
}

#[derive(Clone)]
struct ConsensusVoter {
    config: NodeConfig,
    state: Arc<Mutex<VoterState>>,
    env: Arc<Mutex<Environment>>,
    collect_view: Arc<AtomicU64>,
}

impl ConsensusVoter {
    fn new(
        config: NodeConfig,
        state: Arc<Mutex<VoterState>>,
        env: Arc<Mutex<Environment>>,
        collect_view: Arc<AtomicU64>,
    ) -> Self {
        Self {
            config,
            state,
            env,
            collect_view,
        }
    }

    fn get_leader(view: u64, voters: &VoterSet) -> u64 {
        voters
            .voters
            .get(((view / 100) % voters.voters.len() as u64) as usize)
            .unwrap()
            .to_owned()
    }

    fn new_in_between_block(
        env: Arc<Mutex<Environment>>,
        view: u64,
        generic_qc: QC,
        id: u64,
        lower_bound: usize,
    ) -> Option<NetworkPackage> {
        env.lock()
            .block_tree
            .new_block_with_lowerbound(generic_qc, BlockType::InBetween, lower_bound)
            .map(|blk| Self::package_message(id, Message::ProposeInBetween(blk), view, None))
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

    fn new_key_block(
        env: Arc<Mutex<Environment>>,
        view: u64,
        generic_qc: QC,
        id: u64,
    ) -> NetworkPackage {
        let block = env.lock().block_tree.new_block(generic_qc, BlockType::Key);
        Self::package_message(id, Message::Propose(block), view, None)
    }

    async fn run_as_voter(self) {
        let id = self.state.lock().id;
        let finalized_block_tx = self.env.lock().finalized_block_tx.to_owned();
        let voters = {
            let env = self.env.lock();
            env.voter_set.to_owned()
        };
        // println!("{}: voter start", id);
        let (mut rx, tx) = {
            let mut env = self.env.lock();
            let rx = env.network.take_receiver();
            let tx = env.network.get_sender();
            (rx, tx)
        };
        while let Some(pkg) = rx.recv().await {
            tracing::trace!("{}: voter receive pkg: {:?}", id, pkg);
            let from = pkg.id;
            let view = pkg.view.unwrap();
            if view < self.state.lock().view - 1 {
                continue;
            }
            let message = pkg.message;
            match message {
                Message::Propose(block) => {
                    // TODO: valid block

                    self.env
                        .lock()
                        .block_tree
                        .add_block(block.clone(), BlockType::Key);

                    // TODO: SafeNode
                    let hash = block.hash();
                    let b_x = block.justify.node;
                    let b_y = self
                        .env
                        .lock()
                        .block_tree
                        .get_block(b_x)
                        .unwrap()
                        .0
                        .justify
                        .node;
                    let b_z = self
                        .env
                        .lock()
                        .block_tree
                        .get_block(b_y)
                        .unwrap()
                        .0
                        .justify
                        .node;

                    self.env
                        .lock()
                        .block_tree
                        .add_block(block.to_owned(), BlockType::Key);

                    let current_view = self.state.lock().view;

                    // Suppose the block is valid, vote for it
                    let pkg = Self::package_message(
                        id,
                        Message::Vote(hash),
                        current_view,
                        Some(Self::get_leader(current_view + 1, &voters)),
                    );

                    // let generic_qc = state.lock().generic_qc.to_owned();
                    // let pkg2 =
                    //     Self::new_in_between_block(env.to_owned(), view, generic_qc.to_owned(), id)
                    //         .await;

                    tx.send(pkg).await.unwrap();

                    let is_parent = self.env.lock().block_tree.is_parent(b_x, hash);
                    if is_parent {
                        // Precommit
                        self.state.lock().generic_qc = block.justify.clone();
                        let is_parent = self.env.lock().block_tree.is_parent(b_y, b_x);
                        if is_parent {
                            // Commit
                            self.state.lock().locked_qc = self
                                .env
                                .lock()
                                .block_tree
                                .get_block(b_x)
                                .unwrap()
                                .0
                                .justify
                                .clone();
                            let is_parent = self.env.lock().block_tree.is_parent(b_z, b_y);
                            // if id == 1 {
                            //     println!("{}: {} {} is_parent: {}", id, b_z, b_y, is_parent);
                            // }
                            if is_parent {
                                // Decide/Finalize
                                let finalized_blocks = self.env.lock().block_tree.finalize(b_z);
                                if let Some(tx) = finalized_block_tx.as_ref() {
                                    for block in finalized_blocks {
                                        tx.send(block).await.unwrap();
                                    }
                                }
                            }
                        }
                    }

                    // Finish the view
                    self.state.lock().view_add_one();
                    tracing::trace!("{}: voter finish view: {}", id, current_view);
                }
                Message::ProposeInBetween(block) => {
                    // TODO: valid block
                    let _hash = block.hash();

                    // Add block to block tree
                    self.env
                        .lock()
                        .block_tree
                        .add_block(block, BlockType::InBetween);
                }
                Message::Vote(block_hash) => {
                    self.state.lock().add_vote(view, block_hash, from);
                }
            }
            tracing::trace!("waiting!");
        }
    }

    async fn run_as_leader(self) {
        let id = self.state.lock().id;

        // println!("{}: leader start", id);

        loop {
            let tx = self.env.lock().network.get_sender();
            // println!("{}: leader: test", id);
            // TODO: no need to clone.
            let voters = self.env.lock().voter_set.clone();
            let view = self.state.lock().view;

            if self.config.get_test_mode().delay_test && view > 6 {
                exit(0)
            }

            if Self::get_leader(view, &voters) == id {
                // println!("{}: leader: I am the leader of view {}", id, view);
                // qc for in-between blocks
                let generic_qc = { self.state.lock().generic_qc.to_owned() };

                // let timeout = tokio::time::sleep(tokio::time::Duration::from_millis(1000));
                // tokio::pin!(timeout);

                // let fu = futures::future::poll_fn(|cx| Poll::Ready(()));

                // TODO: loop until timeout or get enough votes
                while self.collect_view.load(Ordering::SeqCst) + 1 < view {
                    if let crate::node_config::ConsensusType::Jasmine { minimal_batch_size } =
                        self.config.get_consensus_type()
                    {
                        let pkg = Self::new_in_between_block(
                            self.env.to_owned(),
                            view,
                            generic_qc.to_owned(),
                            id,
                            *minimal_batch_size,
                        );

                        if let Some(pkg) = pkg {
                            tx.send(pkg).await.unwrap();
                        } else {
                            tokio::task::yield_now().await;
                        }
                    }
                }

                let generic_qc = self.state.lock().generic_qc.clone();
                let pkg = Self::new_key_block(self.env.to_owned(), view, generic_qc, id);
                tracing::trace!("{}: leader: send propose", id);

                tx.send(pkg).await.unwrap();
            }

            let notify = self.state.lock().notify.clone();
            // awake if the view is changed.
            notify.notified().await;
        }
    }
}
