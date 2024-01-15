use std::{
    cell::RefCell,
    sync::Arc,
    time::{Duration, Instant},
};

use minieth::bytes::{Address, Bytes32};
use minieth::rpc::{Rpc, RpcError};

use crate::cached_rpc::PartialLogs;
use crate::worker::worker_name;

#[derive(Debug, Clone, Copy)]
enum BlockRangeIncrease {
    Multiplicative,
    Additive,
}

#[derive(Debug)]
struct BlockRange {
    increase: BlockRangeIncrease,
    size: u64,
}

fn range_size(from_block: u64, to_block: u64) -> u64 {
    to_block - from_block + 1
}

impl BlockRange {
    pub fn on_failure(&mut self, from_block: u64, to_block: u64) {
        self.increase = BlockRangeIncrease::Additive;
        self.size = std::cmp::max(range_size(from_block, to_block) / 2, 1);
    }

    pub fn on_success(&mut self, from_block: u64, to_block: u64) {
        if self.size <= range_size(from_block, to_block) {
            match self.increase {
                BlockRangeIncrease::Multiplicative => self.size = self.size.saturating_mul(2),
                BlockRangeIncrease::Additive => self.size = self.size.saturating_add(1),
            }
        }
    }
}

pub struct SmartGetLogs {
    rpc: Arc<Rpc>,
    addresses: Vec<Address>,
    first_topics: Vec<Bytes32>,

    block_range: RefCell<BlockRange>,
}

const SOFT_TIMEOUT: Duration = Duration::from_secs(5);

impl SmartGetLogs {
    pub fn new(rpc: Arc<Rpc>, addresses: Vec<Address>, first_topics: Vec<Bytes32>) -> Self {
        Self {
            rpc,
            addresses,
            first_topics,
            block_range: RefCell::new(BlockRange {
                increase: BlockRangeIncrease::Multiplicative,
                size: 1,
            }),
        }
    }

    pub fn get_partial_logs(&self, from_block: u64, to_block: u64) -> anyhow::Result<PartialLogs> {
        anyhow::ensure!(
            from_block <= to_block,
            "from_block {from_block} must be <= to_block {to_block}",
        );

        let max_to_block = to_block;

        let mut block_range = self.block_range.borrow_mut();
        loop {
            let to_block = std::cmp::min(
                to_block,
                from_block
                    .saturating_add(block_range.size)
                    .saturating_sub(1),
            );
            let rpc_timer = Instant::now();
            match self
                .rpc
                .get_logs(from_block, to_block, &self.addresses, &self.first_topics)
            {
                Ok(rpc_logs) => {
                    tracing::debug!(
                        from_block,
                        to_block,
                        max_to_block,
                        ?block_range,
                        elapsed=?rpc_timer.elapsed(),
                        "{}: 🚀 rpc getlogs succeeded",
                        worker_name(),
                    );

                    if rpc_timer.elapsed() > SOFT_TIMEOUT {
                        block_range.on_failure(from_block, to_block);
                    } else {
                        block_range.on_success(from_block, to_block);
                    }

                    break Ok(PartialLogs {
                        to_block,
                        logs: rpc_logs,
                    });
                }

                Err(e) => {
                    tracing::warn!(
                        from_block,
                        to_block,
                        max_to_block,
                        ?block_range,
                        elapsed=?rpc_timer.elapsed(),
                        error=?e,
                        "{}: ❌ rpc getlogs failed",
                        worker_name(),
                    );

                    match e {
                        RpcError::ErrorResponse(_)
                        | RpcError::InternalError(_)
                        | RpcError::SerdeJsonError(_) => {
                            if block_range.size == 1 {
                                break Err(e.into());
                            } else {
                                block_range.on_failure(from_block, to_block);
                            }
                        }
                        RpcError::NotFound
                        | RpcError::InvalidInput(_)
                        | RpcError::UnexpectedChainId { .. } => break Err(e.into()),
                    }
                }
            }
        }
    }
}
