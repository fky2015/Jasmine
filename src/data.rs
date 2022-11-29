use std::collections::HashMap;

use crate::crypto::{hash, Digest, Keypair, PublicKey, Signature};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::{debug, error, trace};

// use blake3::{hash, Hash};
use crate::{client::FakeClient, mempool::Mempool, node_config::NodeConfig};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CommandType {
    Set(usize, i64),
    Add(usize, i64),
    Transfer(usize, usize, i64),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Command {
    pub(crate) id: usize,
    pub(crate) created_time: u64,
    pub(crate) command_type: CommandType,
}

impl Command {
    pub(crate) fn serialize(&self, size: usize) -> Vec<u8> {
        let data = bincode::serialize(&self).unwrap();

        let mut bytes = Vec::with_capacity(size);
        bytes.push(data.len() as u8);
        bytes.extend(data);
        bytes.extend(vec![0; size - bytes.len()]);

        bytes
    }

    fn deserialize(bytes: &[u8]) -> Self {
        let size = bytes[0] as usize;
        let data = &bytes[1..size + 1];
        bincode::deserialize(data).unwrap()
    }
}

pub struct CommandGenerator {
    index: usize,
    command_size: usize,
    batch_size: usize,
}

impl CommandGetter for CommandGenerator {
    fn get_commands(&mut self) -> Vec<Transaction> {
        let mut commands = Vec::new();
        for _ in 0..self.batch_size {
            let command = Command {
                id: self.index,
                created_time: 0,
                command_type: CommandType::Set(0, 0),
            };
            commands.push(command.serialize(self.command_size).into());
            self.index += 1;
        }
        commands
    }

    fn get_commands_with_lowerbound(&mut self, _minimal: usize) -> Option<Vec<Transaction>> {
        Some(self.get_commands())
    }
}

pub(crate) trait CommandGetter: Send + Sync {
    /// Try best to get commands from the source.
    fn get_commands(&mut self) -> Vec<Transaction>;

    /// Try best to get commands from the source.
    ///
    /// If commands less than minimal, return None.
    fn get_commands_with_lowerbound(&mut self, minimal: usize) -> Option<Vec<Transaction>>;
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QC {
    pub(crate) node: Digest,
    pub view: u64,
}

impl QC {
    pub(crate) fn new(node: Digest, view: u64) -> Self {
        Self { node, view }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub digest: Digest,
    pub payload: Vec<u8>,
}

impl From<Vec<u8>> for Transaction {
    fn from(payload: Vec<u8>) -> Self {
        let digest = hash(&payload);
        Self { digest, payload }
    }
}

impl Transaction {
    pub fn into_command(self) -> Command {
        Command::deserialize(&self.payload)
    }
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Block {
    pub hash: Digest,
    pub height: usize,
    pub(crate) prev_hash: Digest,
    pub justify: QC,
    pub payloads: Vec<Transaction>,
    pub timestamp: u64,
    pub author: PublicKey,
    pub signature: Signature,
}

impl std::fmt::Debug for Block {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Block")
            .field("height", &self.height)
            .field("prev_hash", &self.prev_hash)
            .field("justify", &self.justify)
            .field("payloads", &self.payloads.len())
            .field("timestamp", &self.timestamp)
            .field("author", &self.author)
            .finish()
    }
}

impl Block {
    pub fn genesis() -> Self {
        Self::default()
    }

    pub fn hash(&self) -> Digest {
        self.hash
    }

    pub fn new(
        prev_hash: Digest,
        prev_height: usize,
        justify: QC,
        payloads: Vec<Transaction>,
        author: PublicKey,
        private_key: &Keypair,
    ) -> Self {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let mut hasher = blake3::Hasher::new();
        hasher.update(prev_hash.as_bytes());
        hasher.update(justify.node.as_bytes());
        for x in &payloads {
            hasher.update(x.digest.as_bytes());
        }
        let hash = hasher.finalize();
        let hash = Digest::from(hash);

        let signature = private_key.sign(&hash);

        Self {
            signature,
            author,
            hash,
            height: prev_height + 1,
            prev_hash,
            justify,
            payloads,
            timestamp,
        }
    }

    pub fn verify(&self) -> Result<()> {
        self.author.verify(&self.hash, &self.signature)?;
        Ok(())
    }
}

#[derive(PartialEq, Eq, Debug, Clone)]
pub(crate) enum BlockType {
    Key,
    InBetween,
}

pub(crate) struct BlockTree {
    pub(crate) config: NodeConfig,
    pub(crate) blocks: HashMap<Digest, (Block, BlockType)>,
    pub(crate) finalized_time: HashMap<Digest, u64>,
    /// latest finalized block (always key block)
    pub(crate) finalized: Digest,
    pub(crate) block_generator: BlockGenerator,
    // pub(crate) finalized_block_tx: Option<Sender<(Block, BlockType, u64)>>,
    pub(crate) parent_key_block: HashMap<Digest, Digest>,
    /// We always make leaf block as a child of latest_key_block.
    /// This is equivalent to the B_leaf in the Pseudocode.
    pub(crate) latest_key_block: Digest,
    /// The latest block we are tracking.
    /// This is always a direct descendant of latest_key_block.
    /// This is used to track in-between blocks.
    pub(crate) latest: Digest,
    pub(crate) enable_metrics: bool,
    /// Tracking all the leaves that is not latest_key_block.
    /// Then we can easily garbage collect the blocks.
    pub(crate) other_leaves: Vec<Digest>,
}

impl BlockTree {
    pub(crate) fn new(genesis: Block, config: &NodeConfig) -> Self {
        let mut blocks = HashMap::new();
        blocks.insert(genesis.hash(), (genesis.to_owned(), BlockType::Key));

        let mut finalized_time = HashMap::new();
        finalized_time.insert(genesis.hash(), 0);

        let mut parent_key_block = HashMap::new();
        parent_key_block.insert(genesis.hash(), genesis.hash());

        Self {
            config: config.clone(),
            blocks,
            latest: genesis.hash(),
            finalized: genesis.hash(),
            finalized_time,
            block_generator: BlockGenerator::new(config),
            parent_key_block,
            latest_key_block: genesis.hash(),
            enable_metrics: false,
            other_leaves: Vec::new(),
        }
    }

    fn get(&self, hash: Digest) -> Option<&(Block, BlockType)> {
        self.blocks.get(&hash)
    }

    fn insert(&mut self, block: Block, block_type: BlockType) {
        // update self.latest if higher
        {
            if self.blocks.contains_key(&block.hash()) {
                trace!(
                    "{}: CONFLICT: self.latest: {} < block: {}, block: {:?}, {:?}",
                    self.config.get_id(),
                    self.latest,
                    block.height,
                    block,
                    self.blocks,
                );
            }
        }
        if self.blocks.get(&self.latest).unwrap().0.height < block.height {
            self.latest = block.hash();
        }

        self.blocks.insert(block.hash(), (block, block_type));
    }

    pub(crate) fn genesis(&self) -> &(Block, BlockType) {
        self.get(Digest::default()).unwrap()
    }

    /// New key block always be built upon latest_key_block.
    pub(crate) fn new_key_block(&mut self, justify: QC) -> Block {
        // if self.latest_key_block is parent key block of self.latest,
        // generate leaf on top of self.latest
        //
        // otherwise, generate leaf on top of self.latest_key_block

        let mut current = self.blocks.get(&self.latest).unwrap();
        while current.1 == BlockType::InBetween {
            current = self.blocks.get(&current.0.prev_hash).unwrap();
        }

        let parent = if current.0.hash() == self.latest_key_block {
            self.latest
        } else {
            // This will lead to a chain fork.
            self.other_leaves.push(self.latest);
            self.latest_key_block
        };
        let prev = &self.get(parent).unwrap().0;
        let block = self.block_generator.new_block(
            prev.hash(),
            prev.height,
            justify,
            self.config.get_id(),
            self.config.get_private_key(),
        );

        // update parent_key_block
        self.parent_key_block
            .insert(block.hash(), self.latest_key_block);
        self.latest_key_block = block.hash();
        self.insert(block.to_owned(), BlockType::Key);
        self.latest = block.hash();

        block
    }

    /// New key block always be built upon latest.
    pub(crate) fn new_in_between_block(&mut self, justify: QC, lowerbound: usize) -> Option<Block> {
        let prev = &self.get(self.latest).unwrap().0;
        let block = self.block_generator.new_block_with_lowerbound(
            prev.hash(),
            prev.height,
            justify,
            lowerbound,
            self.config.get_id(),
            self.config.get_private_key(),
        );

        block.map(|block| {
            // update parent_key_block
            self.insert(block.to_owned(), BlockType::InBetween);
            self.latest = block.hash();

            block
        })
    }

    /// If the block is finalized.
    ///
    /// * `block`: the block to be checked.
    pub(crate) fn finalized(&self, block: Digest) -> bool {
        self.finalized_time.contains_key(&block)
    }

    /// Finalize the block and its ancestors.
    ///
    /// * `block`: the block to be finalized.
    pub(crate) fn finalize(&mut self, block: Digest) -> Vec<(Block, BlockType, u64)> {
        let mut finalized_blocks = Vec::new();
        let finalized_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        // TODO: validate
        if self.blocks.get(&block).unwrap().0.height
            > self.blocks.get(&self.finalized).unwrap().0.height
        {
            self.finalized = block;
        } else {
            return finalized_blocks;
        }

        let mut current = block;
        while !self.finalized(current) {
            self.finalized_time.insert(current, finalized_time);

            if self.enable_metrics {
                let (block, block_type) = self.get(current).unwrap();
                finalized_blocks.push((block.to_owned(), block_type.to_owned(), finalized_time));
            }

            current = self.get(current).unwrap().0.prev_hash;
        }

        if self.finalized_time.len() > self.config.get_node_settings().gc_depth {
            // Start pruning
            let mut current = self.finalized;
            for _ in 0..3 {
                current = match self.parent_key_block.get(&current) {
                    Some(parent) => {
                        // if current == *parent {
                        //     return finalized_blocks;
                        // }

                        *parent
                    }
                    // Should not prune if there is no parent key block.
                    None => return finalized_blocks,
                };
            }
            self.prune(current);
        }

        finalized_blocks
        // println!("finalized: {}", self.finalized_time.len())
    }

    // pub(crate) fn debug_blocks(&self) {
    //     return;
    //     // order by hash
    //     let mut blocks: Vec<(&Hash, &(Block, BlockType))> = self.blocks.iter().collect();
    //     blocks.sort_by(|a, b| a.0.cmp(b.0));
    //     println!("=======");
    //     for (hash, (block, block_type)) in blocks {
    //         println!(
    //             "hash: {}, height: {}, type: {:?}, parent: {}, QC: {:?}",
    //             hash, block.height, block_type, block.prev_hash, block.justify
    //         );
    //     }
    // }

    fn debug_blocks(&self) {
        let mut blocks: Vec<(&Digest, &(Block, BlockType))> = self.blocks.iter().collect();
        blocks.sort_by(|a, b| a.1 .0.height.cmp(&b.1 .0.height));
        println!("=======");
        for (hash, (block, block_type)) in blocks {
            println!(
                "hash: {}, height: {}, type: {:?}, parent: {}, QC: {:?}, from: {:?}",
                hash,
                block.height,
                block_type,
                block.prev_hash,
                block.justify,
                block.author.display()
            );
        }
    }

    pub(crate) fn add_block(&mut self, block: Block, block_type: BlockType) {
        // if block.height < self.blocks.get(&self.latest).unwrap().0.height {
        //     self.debug_blocks();
        //     warn!(
        //         "Block height is too low, {}, prev: {:?}",
        //         block.height,
        //         self.blocks.get(&block.prev_hash)
        //     );
        // }
        if block_type == BlockType::Key {
            // Add it to self.parent_key_block
            let mut parent = match self.blocks.get(&block.prev_hash) {
                Some(p) => p,
                None => {
                    self.debug_blocks();
                    error!(
                        "parent not found: block: {}, parent: {}, height: {}, from: {:?}",
                        block.hash(),
                        block.prev_hash,
                        block.height,
                        block.author.display()
                    );
                    panic!("parent not found");
                }
            };
            while parent.1 == BlockType::InBetween {
                parent = self.blocks.get(&parent.0.prev_hash).unwrap();
            }
            self.parent_key_block.insert(block.hash(), parent.0.hash());
        }
        self.insert(block, block_type);
    }

    /// If child is a direct descendant of parent.
    // must be key blocks
    pub(crate) fn is_parent(&self, parent: Digest, child: Digest) -> bool {
        self.parent_key_block.get(&child) == Some(&parent)
        // let mut current = self.get(child).unwrap().0.prev_hash;
        // while current != parent {
        //     let parent_pair = self.get(current).unwrap();
        //     if parent_pair.1 == BlockType::Key && parent_pair.0.prev_hash != parent {
        //         // println!("{} is not the parent of {}", parent, child);
        //         return false;
        //     }
        //     current = parent_pair.0.prev_hash;
        // }
        // true
    }

    /// If child is a descendant of parent.
    // must be key blocks
    pub(crate) fn extends(&self, parent: Digest, child: Digest) -> bool {
        let mut current = child;
        while current != parent {
            let current_height = match self.blocks.get(&current) {
                Some(pair) => pair.0.height,
                None => {
                    // self.debug_blocks();
                    trace!("block not found: block: {}, child: {}", current, child);
                    return false;
                    // panic!()
                }
            };
            if current_height <= self.blocks.get(&parent).unwrap().0.height {
                // Not in the same chain
                return false;
            }
            current = match self.parent_key_block.get(&current) {
                Some(parent) => *parent,
                // Should not prune if there is no parent key block.
                None => return false,
            };
        }
        true
    }

    pub(crate) fn get_block(&self, hash: Digest) -> Option<&(Block, BlockType)> {
        self.get(hash)
    }

    pub(crate) fn enable_metrics(&mut self) {
        self.enable_metrics = true;
    }

    fn prune(&mut self, block: Digest) {
        let mut current = block;
        if !self.finalized(current) {
            return;
        }
        // Retain only if the leaf is not finalized and we saved the whole block.
        self.other_leaves.retain(|&hash| {
            self.blocks.contains_key(&hash)
                && !self.finalized_time.contains_key(&hash)
                && hash != self.latest
        });

        // Prepare to delete those whose leaf is lower than the target block.
        #[allow(clippy::needless_collect)]
        let drained_leaves = self
            .other_leaves
            .drain_filter(|hash| {
                debug!(
                    "left: {} < right {}, {:?}",
                    hash,
                    block,
                    self.blocks.get(&block)
                );
                self.blocks.get(hash).unwrap().0.height < self.blocks.get(&block).unwrap().0.height
            })
            .collect::<Vec<_>>();

        // Delete blocks info until find a finalized block or empty.
        drained_leaves.into_iter().for_each(|mut current| {
            while !self.finalized(current) {
                let parent = match self.get(current) {
                    Some((block, _)) => block.prev_hash,
                    None => break,
                };
                self.blocks.remove(&current);
                current = parent;
            }
        });

        while let Some((_, (block, _))) = self.blocks.remove_entry(&current) {
            self.finalized_time.remove(&current);
            self.parent_key_block.remove(&current);
            current = block.prev_hash;
        }
    }

    fn get_parent_key_block(&self, in_between: Digest) -> Digest {
        let mut current = self.get(in_between).unwrap();
        while current.1 != BlockType::Key {
            current = self.get(current.0.prev_hash).unwrap();
        }

        current.0.hash()
    }

    pub(crate) fn switch_latest_key_block(&mut self, node: Digest) {
        // if self.latest_key_block, self.latest,
        // and node is on the same chain, then there is no fork.

        if self.extends(self.latest_key_block, node) {
            let latest_key = self.get_parent_key_block(self.latest);

            if self.extends(node, latest_key) || self.extends(latest_key, node) {
                self.latest_key_block = node;
            } else {
                self.latest_key_block = node;
                self.other_leaves.push(self.latest);
                self.latest = node;
            }
        } else {
            self.latest_key_block = node;
            self.other_leaves.push(self.latest);
        }
    }
}

pub(crate) struct BlockGenerator {
    mempool: Box<dyn CommandGetter + Send>,
}

impl BlockGenerator {
    fn new(config: &NodeConfig) -> Self {
        if config.get_client_config().use_instant_generator {
            Self {
                mempool: Box::new(CommandGenerator {
                    index: 0,
                    command_size: config.get_node_settings().transaction_size,
                    batch_size: config.get_node_settings().batch_size,
                }),
            }
        } else {
            let (mempool, tx) = Mempool::spawn_receiver(config.to_owned());
            FakeClient::spawn(config.to_owned(), tx);

            Self {
                mempool: Box::new(mempool),
            }
        }
    }

    fn new_block(
        &mut self,
        prev_hash: Digest,
        prev_height: usize,
        justify: QC,
        author: PublicKey,
        priv_key: &Keypair,
    ) -> Block {
        self.new_block_with_lowerbound(prev_hash, prev_height, justify, 0, author, priv_key)
            .unwrap()
    }

    fn new_block_with_lowerbound(
        &mut self,
        prev_hash: Digest,
        prev_height: usize,
        justify: QC,
        lowerbound: usize,
        author: PublicKey,
        priv_key: &Keypair,
    ) -> Option<Block> {
        let payloads = self.mempool.get_commands_with_lowerbound(lowerbound);
        payloads
            .map(|payloads| Block::new(prev_hash, prev_height, justify, payloads, author, priv_key))
    }
}

//test
#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_command_serialization() {
        {
            let command = Command {
                id: 1,
                created_time: 2,
                command_type: CommandType::Set(1, 2),
            };

            let serialized = command.serialize(256);

            let deserialized = Command::deserialize(&serialized);
            assert_eq!(command, deserialized);
            assert_eq!(serialized.capacity(), 256)
        }

        {
            let command = Command {
                //max
                id: usize::MAX,
                created_time: u64::MAX,
                command_type: CommandType::Add(usize::MAX, i64::MAX),
            };
            let serialized = command.serialize(256);

            let deserialized = Command::deserialize(&serialized);
            assert_eq!(serialized.len(), 256);
            assert_eq!(command, deserialized);
            assert_eq!(serialized.capacity(), 256);
        }
    }

    #[test]
    fn different_sized_tx() {
        let command = Command {
            //max
            id: usize::MAX,
            created_time: u64::MAX,
            command_type: CommandType::Add(usize::MAX, i64::MAX),
        };
        assert_eq!(command.serialize(128).len(), 128);
        assert_eq!(command.serialize(256).len(), 256);
        assert_eq!(command.serialize(512).len(), 512);
        assert_eq!(command.serialize(1024).len(), 1024);
    }
}
