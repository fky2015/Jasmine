use crate::{
    crypto::{Digest, PublicKey, Signature},
    data::{BlockType, QC},
    node_config::NodeConfig,
};
use std::{
    collections::HashMap,
    slice::Iter,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use tokio::sync::{mpsc::Sender, Notify};

use serde::{Deserialize, Serialize};

use parking_lot::Mutex;
use tracing::{debug, trace, warn};

use crate::{
    data::{Block, BlockTree},
    network::MemoryNetworkAdaptor,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum Message {
    Propose(Block),
    ProposeInBetween(Block),
    Vote(Digest, PublicKey, Signature),
    NewView(QC),
}

impl Message {}

pub(crate) struct VoterState {
    pub id: PublicKey,
    pub view: u64,
    pub threshold: usize,
    pub generic_qc: QC,
    pub locked_qc: QC,
    // <view, (what, whos)>
    pub votes: HashMap<u64, HashMap<Digest, Vec<PublicKey>>>,
    pub notify: Arc<Notify>,
    pub best_view: Arc<AtomicU64>,
    // <view, (whos)>
    pub new_views: HashMap<u64, Vec<PublicKey>>,
}

impl VoterState {
    pub fn new(id: PublicKey, view: u64, generic_qc: QC, threshold: usize) -> Self {
        Self {
            id,
            view,
            threshold,
            generic_qc: generic_qc.to_owned(),
            locked_qc: generic_qc,
            votes: HashMap::new(),
            notify: Arc::new(Notify::new()),
            best_view: Arc::new(AtomicU64::new(0)),
            new_views: HashMap::new(),
        }
    }

    pub(crate) fn view_add_one(&mut self) {
        // println!("{}: view add to {}", self.id, self.view + 1);
        // Prune old votes
        self.votes.retain(|v, _| v >= &self.view);
        self.new_views.retain(|v, _| v >= &self.view);

        self.view += 1;
        self.notify.notify_waiters();
    }

    pub(crate) fn add_new_view(&mut self, view: u64, who: PublicKey) {
        let view_map = self.new_views.entry(view).or_default();
        // TODO, use a hashmap to collect messages.
        view_map.push(who);

        if view_map.len() == self.threshold {
            trace!(
                "{}: new view {} is ready, current: {}",
                self.id,
                view,
                self.view
            );
            self.best_view.store(view, Ordering::SeqCst);
            self.notify.notify_waiters();
        }
    }

    // return whether a new qc formed.
    pub(crate) fn add_vote(
        &mut self,
        msg_view: u64,
        block_hash: Digest,
        voter_id: PublicKey,
    ) -> Option<QC> {
        let view_map = self.votes.entry(msg_view).or_default();
        let voters = view_map.entry(block_hash).or_default();
        // TODO: check if voter_id is already in voters
        voters.push(voter_id);

        if voters.len() == self.threshold {
            trace!(
                "{}: Vote threshould {} is ready, current: {}",
                self.id,
                msg_view,
                self.view
            );
            Some(QC::new(block_hash, msg_view))
        } else {
            trace!(
                "{}: Vote threshould {} is not ready, current: {}, threadhold: {}",
                self.id,
                msg_view,
                self.view,
                self.threshold
            );
            None
        }
    }

    pub(crate) fn set_best_view(&mut self, view: u64) {
        self.best_view.store(view, Ordering::Relaxed);
    }

    pub(crate) fn best_view_ref(&self) -> Arc<AtomicU64> {
        self.best_view.to_owned()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NetworkPackage {
    pub(crate) from: PublicKey,
    /// None means the message is a broadcast message.
    pub(crate) to: Option<PublicKey>,
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
    id: PublicKey,
    config: NodeConfig,
    /// Only used when initialize ConsensusVoter.
    view: u64,
    env: Arc<Mutex<Environment>>,
}

#[derive(Debug, Clone)]
pub(crate) struct VoterSet {
    voters: Vec<PublicKey>,
}

impl VoterSet {
    pub fn new(voters: Vec<PublicKey>) -> Self {
        Self { voters }
    }

    pub fn threshold(&self) -> usize {
        self.voters.len() - (self.voters.len() as f64 / 3.0).floor() as usize
    }

    pub fn iter(&self) -> Iter<PublicKey> {
        self.voters.iter()
    }
}

impl Iterator for VoterSet {
    type Item = PublicKey;

    fn next(&mut self) -> Option<Self::Item> {
        self.voters.pop()
    }
}

impl Voter {
    pub(crate) fn new(id: PublicKey, config: NodeConfig, env: Arc<Mutex<Environment>>) -> Self {
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
        let pacemaker = voter.clone();

        let handler1 = tokio::spawn(async {
            leader.run_as_leader().await;
        });

        let handler2 = tokio::spawn(async {
            voter.run_as_voter().await;
        });

        let handler3 = tokio::spawn(async {
            pacemaker.run_as_pacemaker().await;
        });

        tokio::join!(handler1, handler2, handler3);
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

    fn get_leader(view: u64, voters: &VoterSet, leader_rotation: usize) -> PublicKey {
        voters
            .voters
            .get(((view / leader_rotation as u64) % voters.voters.len() as u64) as usize)
            .unwrap()
            .to_owned()
    }

    fn new_in_between_block(
        env: Arc<Mutex<Environment>>,
        view: u64,
        generic_qc: QC,
        id: PublicKey,
        lower_bound: usize,
    ) -> Option<NetworkPackage> {
        env.lock()
            .block_tree
            .new_in_between_block(generic_qc, lower_bound)
            .map(|blk| Self::package_message(id, Message::ProposeInBetween(blk), view, None))
    }

    fn package_message(
        id: PublicKey,
        message: Message,
        view: u64,
        to: Option<PublicKey>,
    ) -> NetworkPackage {
        NetworkPackage {
            from: id,
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
        id: PublicKey,
    ) -> NetworkPackage {
        let block = env.lock().block_tree.new_key_block(generic_qc);
        Self::package_message(id, Message::Propose(block), view, None)
    }

    fn update_qc_high(&self, new_qc: QC) -> bool {
        let mut state = self.state.lock();
        if new_qc.view > state.generic_qc.view {
            debug!(
                "Node {} update qc_high from {:?} to {:?}",
                self.config.get_id(),
                state.generic_qc,
                new_qc
            );
            state.generic_qc = new_qc.to_owned();
            drop(state);
            self.env
                .lock()
                .block_tree
                .switch_latest_key_block(new_qc.node);
            true
        } else {
            false
        }
    }

    async fn run_as_voter(self) {
        let id = self.state.lock().id;
        let finalized_block_tx = self.env.lock().finalized_block_tx.to_owned();
        let leader_rotation = self.config.get_node_settings().leader_rotation;
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

        // The view voted for last block.
        //
        // Initialize as 0, since we voted for genesis block.
        let mut voted_view = 0;

        while let Some(pkg) = rx.recv().await {
            let from = pkg.from;
            let view = pkg.view.unwrap();
            let current_view = self.state.lock().view;
            if view < current_view - 1 {
                // trace!(
                //     "{}: voter receive pkg from {} with view {} which is too old, pkg: {:?}",
                //     id,
                //     from,
                //     view,
                //     pkg
                // );
                continue;
            }
            let message = pkg.message;
            match message {
                Message::Propose(block) => {
                    if view < self.state.lock().view {
                        continue;
                    }
                    let hash = block.hash();

                    // WARN: As a POC, we suppose all the blocks are valid by application logic.
                    block.verify().unwrap();

                    if from != id {
                        self.env
                            .lock()
                            .block_tree
                            .add_block(block.clone(), BlockType::Key);
                    }

                    // onReceiveProposal
                    if let Some(pkg) = {
                        let locked_qc = self.state.lock().locked_qc.clone();
                        let safety = self
                            .env
                            .lock()
                            .block_tree
                            .extends(locked_qc.node, block.hash());
                        let liveness = block.justify.view >= locked_qc.view;

                        if view > voted_view && (safety || liveness) {
                            voted_view = view;

                            // Suppose the block is valid, vote for it
                            Some(Self::package_message(
                                id,
                                Message::Vote(hash, id, self.config.sign(&hash)),
                                current_view,
                                Some(Self::get_leader(current_view + 1, &voters, leader_rotation)),
                            ))
                        } else {
                            trace!(
                                "{}: Safety: {} or Liveness: {} are both invalid",
                                id,
                                safety,
                                liveness
                            );
                            None
                        }
                    } {
                        trace!("{}: send vote {:?} for block", id, pkg);
                        tx.send(pkg).await.unwrap();
                    }

                    // update
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

                    trace!("{}: enter PRE-COMMIT phase", id);
                    // PRE-COMMIT phase on b_x
                    self.update_qc_high(block.justify.clone());

                    let larger_view = self
                        .env
                        .lock()
                        .block_tree
                        .get_block(b_x)
                        .unwrap()
                        .0
                        .justify
                        .view
                        > self.state.lock().locked_qc.view;
                    if larger_view {
                        trace!("{}: enter COMMIT phase", id);
                        // COMMIT phase on b_y
                        self.state.lock().locked_qc = self
                            .env
                            .lock()
                            .block_tree
                            .get_block(b_x)
                            .unwrap()
                            .0
                            .justify
                            .clone();
                    }

                    let is_parent = self.env.lock().block_tree.is_parent(b_y, b_x);
                    if is_parent {
                        let is_parent = self.env.lock().block_tree.is_parent(b_z, b_y);
                        if is_parent {
                            trace!("{}: enter DECIDE phase", id);
                            // DECIDE phase on b_z / Finalize b_z
                            let finalized_blocks = self.env.lock().block_tree.finalize(b_z);
                            // onCommit
                            if let Some(tx) = finalized_block_tx.as_ref() {
                                for block in finalized_blocks {
                                    tx.send(block).await.unwrap();
                                }
                            }
                        }
                    }

                    trace!("{}: view add one", id);
                    // Finish the view
                    self.state.lock().view_add_one();

                    tracing::trace!("{}: voter finish view: {}", id, current_view);
                }
                Message::ProposeInBetween(block) => {
                    // TODO: valid block
                    let _hash = block.hash();

                    if from != id {
                        // Add block to block tree
                        self.env
                            .lock()
                            .block_tree
                            .add_block(block, BlockType::InBetween);
                    }
                }
                Message::Vote(block_hash, author, signature) => {
                    // onReceiveVote
                    let qc = self.state.lock().add_vote(view, block_hash, from);

                    // verify signature
                    author.verify(&block_hash, &signature).unwrap();

                    if let Some(qc) = qc {
                        self.update_qc_high(qc);
                        self.state.lock().set_best_view(view);
                    }
                }
                Message::NewView(high_qc) => {
                    self.update_qc_high(high_qc);
                    self.state.lock().add_new_view(view, from);
                }
            }
        }
    }

    async fn run_as_leader(self) {
        let id = self.state.lock().id;
        let leader_rotation = self.config.get_node_settings().leader_rotation;

        // println!("{}: leader start", id);

        loop {
            let tx = self.env.lock().network.get_sender();
            // TODO: no need to clone.
            let voters = self.env.lock().voter_set.clone();
            let view = self.state.lock().view;

            if Self::get_leader(view, &voters, leader_rotation) == id {
                tracing::trace!("{}: start as leader in view: {}", id, view);
                // qc for in-between blocks
                let generic_qc = { self.state.lock().generic_qc.to_owned() };

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
                    } else {
                        tokio::task::yield_now().await;
                    }
                }

                // onPropose
                let generic_qc = self.state.lock().generic_qc.clone();
                let pkg = Self::new_key_block(self.env.to_owned(), view, generic_qc, id);
                tracing::trace!("{}: leader propose block in view: {}", id, view);
                tx.send(pkg).await.unwrap();
            }

            let notify = self.state.lock().notify.clone();
            // Get awoke if the view is changed.
            notify.notified().await;
            {
                let view = self.state.lock().view;
                trace!(
                    "{}: leader notified, view: {}, leader: {}",
                    id,
                    view,
                    Self::get_leader(view, &voters, leader_rotation)
                );
            }
        }
    }

    async fn run_as_pacemaker(self) {
        let timeout = tokio::time::Duration::from_millis(1000);
        let tx = self.env.lock().network.get_sender();
        let id = self.config.get_id();

        loop {
            let past_view = self.state.lock().view;
            let next_awake = tokio::time::Instant::now() + timeout;
            trace!("{}: pacemaker start", id);
            tokio::time::sleep_until(next_awake).await;
            trace!("{}: pacemaker awake", id);

            // If last vote is received later then 1s ago, then continue to sleep.
            let current_view = self.state.lock().view;
            if current_view != past_view {
                continue;
            }
            warn!("{} timeout!!!", id);

            // otherwise, try send a new-view message to nextleader
            let (next_leader, next_leader_view) = self.get_next_leader();
            trace!("{} send new_view to {}", id, next_leader);
            let pkg = self.new_new_view(next_leader_view, next_leader);
            tx.send(pkg).await.unwrap();

            self.state.lock().view = next_leader_view;
        }
    }

    fn new_new_view(&self, view: u64, next_leader: PublicKey) -> NetworkPackage {
        let new_view = Message::NewView(self.state.lock().generic_qc.clone());
        Self::package_message(self.state.lock().id, new_view, view, Some(next_leader))
    }

    // -> (leaderId, view)
    fn get_next_leader(&self) -> (PublicKey, u64) {
        let mut view = self.state.lock().view;
        let leader_rotation = self.config.get_node_settings().leader_rotation;
        let current_leader = Self::get_leader(view, &self.env.lock().voter_set, leader_rotation);
        loop {
            view += 1;
            let next_leader = Self::get_leader(view, &self.env.lock().voter_set, leader_rotation);
            if next_leader != current_leader {
                return (next_leader, view);
            }
        }
    }
}
