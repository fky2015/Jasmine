use std::collections::HashMap;

use serde::{Deserialize, Serialize};

pub type Hash = u64;

// in bytes
const COMMAND_SIZE: usize = 256;
const BATCH_SIZE: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CommandType {
    Set(usize, i64),
    Add(usize, i64),
    Transfer(usize, usize, i64),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Command {
    id: usize,
    created_time: u64,
    command_type: CommandType,
}

impl Command {
    fn serialize(&self) -> [u8; COMMAND_SIZE] {
        let serialized = serde_json::to_string(self).unwrap();
        let mut bytes = [0u8; COMMAND_SIZE];
        bytes[..serialized.as_bytes().len()].copy_from_slice(serialized.as_bytes());
        bytes
    }

    fn deserialize(bytes: &[u8; COMMAND_SIZE]) -> Self {
        let serialized = String::from_utf8(bytes.to_vec())
            .unwrap()
            .trim_end_matches(char::from(0))
            .to_string();
        serde_json::from_str(&serialized).unwrap()
    }
}

pub struct CommandGenerator {
    index: usize,
}

impl CommandGetter for CommandGenerator {
    fn get_command(&mut self, batch_size: usize) -> Vec<[u8; COMMAND_SIZE]> {
        let mut commands = Vec::new();
        for _ in 0..batch_size {
            let command = Command {
                id: self.index,
                created_time: 0,
                command_type: CommandType::Set(0, 0),
            };
            commands.push(command.serialize());
            self.index += 1;
        }
        commands
    }
}

trait CommandGetter: Send + Sync {
    /// Try best to get commands from the source.
    fn get_command(&mut self, batch_size: usize) -> Vec<[u8; COMMAND_SIZE]>;
}

pub struct NodeConfig {
    mempool_size: usize,
}

impl NodeConfig {
    fn default() -> Self {
        Self { mempool_size: 100 }
    }
}

#[derive(Debug, Clone)]
pub struct QC {
    pub node: Hash,
}

#[derive(Clone)]
pub struct Block {
    pub height: usize,
    pub prev_hash: Hash,
    pub justify: QC,
    pub payloads: Vec<[u8; COMMAND_SIZE]>,
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
            justify: QC { node: 0 },
            payloads: Vec::new(),
        }
    }

    pub fn hash(&self) -> Hash {
        self.height as Hash
    }

    pub fn new(
        prev_hash: Hash,
        prev_height: usize,
        justify: QC,
        payloads: Vec<[u8; COMMAND_SIZE]>,
    ) -> Self {
        Self {
            height: prev_height + 1,
            prev_hash: prev_hash,
            justify,
            payloads,
        }
    }
}

pub(crate) enum BlockType {
    Key,
    InBetween,
}

pub(crate) struct BlockTree {
    pub(crate) blocks: HashMap<Hash, (Block, BlockType)>,
    pub(crate) finalized_time: HashMap<Hash, u64>,
    pub(crate) latest: Hash,
    pub(crate) finalized: Hash,
    pub(crate) block_generator: BlockGenerator,
}

impl BlockTree {
    pub(crate) fn new(genesis: Block) -> Self {
        let mut blocks = HashMap::new();
        blocks.insert(genesis.hash(), (genesis.to_owned(), BlockType::Key));
        Self {
            blocks,
            latest: genesis.hash(),
            finalized: genesis.hash(),
            finalized_time: HashMap::new(),
            block_generator: BlockGenerator::new(),
        }
    }

    fn get(&self, hash: Hash) -> Option<&(Block, BlockType)> {
        self.blocks.get(&hash)
    }

    fn insert(&mut self, block: Block, block_type: BlockType) {
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
        let prev = &self.get(self.latest).unwrap().0;
        let block = self
            .block_generator
            .new_block(prev.hash(), prev.height, justify);
        self.insert(block.to_owned(), block_type);
        self.latest = block.hash();
        block
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
    pub(crate) fn finalize(&mut self, block: Hash) {
        let finalized_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let mut current = block;
        while !self.finalized(current) {
            self.finalized_time.insert(current, finalized_time);
            current = self.get(current).unwrap().0.prev_hash;
        }
    }

    pub(crate) fn add_block(&mut self, block: Block, block_type: BlockType) {
        if self.get(block.hash()).is_none() {
            self.insert(block, block_type);
        }
    }
}

pub(crate) struct BlockGenerator {
    mempool: Box<dyn CommandGetter + Send>,
    genesis: Block,
}

impl BlockGenerator {
    fn new() -> Self {
        Self {
            mempool: Box::new(CommandGenerator { index: 0 }),
            genesis: Block::genesis(),
        }
    }

    fn new_block(&mut self, prev_hash: Hash, prev_height: usize, justify: QC) -> Block {
        let payloads = self.mempool.get_command(BATCH_SIZE);
        Block::new(prev_hash, prev_height, justify, payloads)
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
            let serialized = command.serialize();

            let deserialized = Command::deserialize(&serialized);
            assert_eq!(command, deserialized);
        }

        {
            let command = Command {
                //max
                id: usize::MAX,
                created_time: u64::MAX,
                command_type: CommandType::Add(usize::MAX, i64::MAX),
            };
            let serialized = command.serialize();

            let deserialized = Command::deserialize(&serialized);
            assert_eq!(command, deserialized);
        }
    }
}
