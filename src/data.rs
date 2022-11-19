use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_big_array::BigArray;

use crate::{client::FakeClient, mempool::Mempool, node_config::NodeConfig};

pub type Hash = u64;

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
        let serialized = serde_json::to_string(self).unwrap();
        let mut bytes = Vec::with_capacity(size);
        bytes.extend_from_slice(serialized.as_bytes());
        bytes
    }

    fn deserialize(bytes: &Vec<u8>) -> Self {
        let serialized = String::from_utf8(bytes.to_vec())
            .unwrap()
            .trim_end_matches(char::from(0))
            .to_string();
        serde_json::from_str(&serialized).unwrap()
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
        return Some(self.get_commands());
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QC {
    pub node: Hash,
    pub view: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub payload: Vec<u8>,
}

impl From<Vec<u8>> for Transaction {
    fn from(payload: Vec<u8>) -> Self {
        Self { payload }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Block {
    pub height: usize,
    pub prev_hash: Hash,
    pub justify: QC,
    // #[serde(with = "BigArray")]
    pub payloads: Vec<Transaction>,
    pub timestamp: u64,
}

impl std::fmt::Debug for Block {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Block")
            .field("height", &self.height)
            .field("prev_hash", &self.prev_hash)
            .field("justify", &self.justify)
            .field("payloads", &self.payloads.len())
            .finish()
    }
}

impl Block {
    pub fn genesis() -> Self {
        Self {
            height: 0,
            prev_hash: 0,
            justify: QC { node: 0, view: 0 },
            payloads: Vec::new(),
            timestamp: 0,
        }
    }

    pub fn hash(&self) -> Hash {
        self.height as Hash
    }

    pub fn new(
        prev_hash: Hash,
        prev_height: usize,
        justify: QC,
        payloads: Vec<Transaction>,
    ) -> Self {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        Self {
            height: prev_height + 1,
            prev_hash,
            justify,
            payloads,
            timestamp,
        }
    }
}

#[derive(PartialEq, Eq, Debug, Clone)]
pub(crate) enum BlockType {
    Key,
    InBetween,
}

pub(crate) struct BlockTree {
    pub(crate) blocks: HashMap<Hash, (Block, BlockType)>,
    pub(crate) finalized_time: HashMap<Hash, u64>,
    pub(crate) latest: Hash,
    /// latest finalized block (always key block)
    pub(crate) finalized: Hash,
    pub(crate) block_generator: BlockGenerator,
    // pub(crate) finalized_block_tx: Option<Sender<(Block, BlockType, u64)>>,
    pub(crate) parent_key_block: HashMap<Hash, Hash>,
    pub(crate) latest_key_block: Hash,
    pub(crate) enable_metrics: bool,
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
            blocks,
            latest: genesis.hash(),
            finalized: genesis.hash(),
            finalized_time,
            block_generator: BlockGenerator::new(config),
            parent_key_block,
            latest_key_block: genesis.hash(),
            enable_metrics: false,
        }
    }

    fn get(&self, hash: Hash) -> Option<&(Block, BlockType)> {
        self.blocks.get(&hash)
    }

    fn insert(&mut self, block: Block, block_type: BlockType) {
        // update latest
        if self.blocks.get(&self.latest).unwrap().0.height < block.height {
            self.latest = block.hash();
        }

        self.blocks.insert(block.hash(), (block, block_type));
    }

    pub(crate) fn genesis(&self) -> &(Block, BlockType) {
        self.get(0).unwrap()
    }

    // TODO:
    /// Generate a new block based on the latest block.
    ///
    /// * `justify`:
    pub(crate) fn new_block(&mut self, justify: QC, block_type: BlockType) -> Block {
        self.new_block_with_lowerbound(justify, block_type, 0)
            .expect("lowerbound already set to 0")
    }

    pub(crate) fn new_block_with_lowerbound(
        &mut self,
        justify: QC,
        block_type: BlockType,
        lowerbound: usize,
    ) -> Option<Block> {
        let prev = &self.get(self.latest).unwrap().0;
        let block = self.block_generator.new_block_with_lowerbound(
            prev.hash(),
            prev.height,
            justify,
            lowerbound,
        );

        block.map(|block| {
            // update parent_key_block
            if block_type == BlockType::Key {
                self.parent_key_block
                    .insert(block.hash(), self.latest_key_block);
                self.latest_key_block = block.hash();
            }
            self.insert(block.to_owned(), block_type);
            self.latest = block.hash();

            block
        })
    }

    /// If the block is finalized.
    ///
    /// * `block`: the block to be checked.
    pub(crate) fn finalized(&mut self, block: Hash) -> bool {
        self.finalized_time.contains_key(&block)
    }

    /// Finalize the block and its ancestors.
    ///
    /// * `block`: the block to be finalized.
    pub(crate) fn finalize(&mut self, block: Hash) -> Vec<(Block, BlockType, u64)> {
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

        if self.finalized_time.len() > 200 {
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

    pub(crate) fn add_block(&mut self, block: Block, block_type: BlockType) {
        if self.get(block.hash()).is_none() {
            if block_type == BlockType::Key {
                self.parent_key_block
                    .insert(block.hash(), self.latest_key_block);
                self.latest_key_block = block.hash();
            }
            self.insert(block, block_type);
        }
    }

    // must be key blocks
    pub(crate) fn is_parent(&self, parent: Hash, child: Hash) -> bool {
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

    pub(crate) fn get_block(&self, hash: Hash) -> Option<&(Block, BlockType)> {
        self.get(hash)
    }

    pub(crate) fn enable_metrics(&mut self) {
        self.enable_metrics = true;
    }

    fn prune(&mut self, block: Hash) {
        let mut current = block;
        if !self.finalized(current) {
            return;
        }
        while let Some((_, (block, _))) = self.blocks.remove_entry(&current) {
            self.finalized_time.remove(&current);
            self.parent_key_block.remove(&current);
            current = block.prev_hash;
        }
    }
}

pub(crate) struct BlockGenerator {
    mempool: Box<dyn CommandGetter + Send>,
    genesis: Block,
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
                genesis: Block::genesis(),
            }
        } else {
            let (mempool, tx) = Mempool::spawn_receiver(config.to_owned());
            FakeClient::spawn(config.to_owned(), tx);

            Self {
                mempool: Box::new(mempool),
                genesis: Block::genesis(),
            }
        }
    }

    fn new_block(&mut self, prev_hash: Hash, prev_height: usize, justify: QC) -> Block {
        self.new_block_with_lowerbound(prev_hash, prev_height, justify, 0)
            .unwrap()
    }

    fn new_block_with_lowerbound(
        &mut self,
        prev_hash: Hash,
        prev_height: usize,
        justify: QC,
        lowerbound: usize,
    ) -> Option<Block> {
        let payloads = self.mempool.get_commands_with_lowerbound(lowerbound);
        payloads.map(|payloads| Block::new(prev_hash, prev_height, justify, payloads))
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
            assert_eq!(command, deserialized);
            assert_eq!(serialized.capacity(), 256);
        }
    }
}
