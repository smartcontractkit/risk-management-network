use anyhow::{bail, Result};
use core::fmt;
use minieth::rpc::Block;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn age(block_timestamp: u64) -> Duration {
    let epoch_seconds_now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    Duration::from_secs(epoch_seconds_now.saturating_sub(block_timestamp))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainStatus {
    FinalityViolated,
    Good { tail: Tail },
}

impl fmt::Display for ChainStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FinalityViolated => write!(f, "FinalityViolated"),
            Self::Good { tail } => write!(f, "Good({tail})"),
        }
    }
}

impl ChainStatus {
    pub fn tip(&self) -> Option<&Block> {
        match self {
            Self::FinalityViolated => None,
            Self::Good { tail } => Some(tail.tip()),
        }
    }

    pub fn stable_tip(&self) -> Option<&Block> {
        match self {
            Self::FinalityViolated => None,
            Self::Good { tail } => Some(tail.stable_tip()),
        }
    }

    pub fn block_by_number(&self, number: u64) -> Option<&Block> {
        match self {
            Self::FinalityViolated => None,
            Self::Good { tail } => tail.block_by_number(number),
        }
    }

    pub fn contains(&self, header: &Block) -> bool {
        match self {
            Self::FinalityViolated => false,
            Self::Good { tail } => tail.contains(header),
        }
    }

    pub fn age(&self) -> Option<Duration> {
        self.tip().map(|b| age(b.timestamp))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tail(Vec<Block>);

impl fmt::Display for Tail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (stable_tip, tip) = (self.stable_tip(), self.tip());
        write!(
            f,
            "({}, {})..({}, {})",
            stable_tip.number, stable_tip.hash, tip.number, tip.hash,
        )
    }
}

impl Tail {
    pub fn new(blocks: Vec<Block>) -> Result<Self> {
        if blocks.is_empty() {
            bail!("tail can't be empty")
        }
        let mut prev_block_number_and_hash = None;
        for block in blocks.iter() {
            if let Some((prev_block_num, prev_hash)) = prev_block_number_and_hash {
                if prev_block_num == u64::MAX || prev_block_num + 1 != block.number {
                    bail!(
                        "found gap in block number between {} and {} in blocks {:?}",
                        prev_block_num,
                        block.number,
                        blocks,
                    )
                }
                if prev_hash != block.parent_hash {
                    bail!(
                        "found unmatched parent hash, expected {}, got {} in blocks {:?}",
                        block.hash,
                        prev_hash,
                        blocks,
                    )
                }
            }
            prev_block_number_and_hash = Some((block.number, block.hash))
        }
        Ok(Tail(blocks))
    }

    pub fn tip(&self) -> &Block {
        self.0.last().unwrap()
    }

    pub fn stable_tip(&self) -> &Block {
        self.0.first().unwrap()
    }

    fn index_by_number(&self, number: u64) -> Option<usize> {
        number
            .checked_sub(self.stable_tip().number)
            .map(|index| index as usize)
    }

    pub fn block_by_number(&self, number: u64) -> Option<&Block> {
        self.0.get(self.index_by_number(number)?)
    }

    pub fn contains(&self, header: &Block) -> bool {
        self.block_by_number(header.number)
            .map_or(false, |block| block.hash == header.hash)
    }
}
