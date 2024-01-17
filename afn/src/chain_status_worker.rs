use crate::chain_status::{ChainStatus, Tail};
use crate::common::ChainName;
use crate::metrics::ChainMetrics;
use crate::worker::{Context, ShutdownHandle};
use anyhow::{anyhow, bail, Result};
use minieth::bytes::Bytes32;
use minieth::rpc::BlockRpc;
use minieth::rpc::{Block, BlockIdentifier};
use std::collections::{HashMap, VecDeque};
use std::ops::RangeInclusive;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use tracing::{error, info, trace, warn};

pub trait ChainStatusUpdater {
    fn initial(&self) -> Result<ChainStatus>;
    fn update(&self, ctx: &Context, status: &ChainStatus) -> Result<ChainStatus>;
}

pub struct ConfirmationDepthChainStatusUpdater {
    pub soft_confirmations: usize,
    pub hard_confirmations: usize,
    pub rpc: Arc<dyn BlockRpc>,
    pub name: ChainName,
}

impl ConfirmationDepthChainStatusUpdater {
    fn get_block_with_confirmations(&self, confirmations: usize) -> Result<Block> {
        let latest = self.rpc.get_block(BlockIdentifier::Latest)?;
        Ok(self.rpc.get_block(BlockIdentifier::Height(
            latest
                .number
                .saturating_sub(u64::try_from(confirmations).unwrap()),
        ))?)
    }
}

impl ChainStatusUpdater for ConfirmationDepthChainStatusUpdater {
    fn initial(&self) -> Result<ChainStatus> {
        Ok(ChainStatus::Good {
            tail: Tail::new(vec![self.get_block_with_confirmations(
                self.hard_confirmations + self.soft_confirmations,
            )?])?,
        })
    }

    fn update(&self, ctx: &Context, status: &ChainStatus) -> Result<ChainStatus> {
        match status {
            ChainStatus::FinalityViolated => Ok(ChainStatus::FinalityViolated),
            ChainStatus::Good { tail } => {
                let start = Instant::now();

                let mut new_tail = VecDeque::<Block>::new();
                new_tail.push_front(self.get_block_with_confirmations(self.soft_confirmations)?);
                loop {
                    if ctx.is_done() {
                        bail!("update aborted due to shutdown signal")
                    }
                    let new_tail_head = new_tail.front().unwrap();
                    if tail.contains(new_tail_head) {
                        break;
                    } else if new_tail_head.number == tail.stable_tip().number {
                        tracing::error!(chain=%self.name, ?tail, ?new_tail, "finality violated: confirmation depth");
                        return Ok(ChainStatus::FinalityViolated);
                    } else if new_tail_head.number < tail.stable_tip().number {
                        return Ok(ChainStatus::Good { tail: tail.clone() });
                    }
                    new_tail.push_front(
                        self.rpc
                            .get_block(BlockIdentifier::Hash(new_tail_head.parent_hash))?,
                    );
                }

                while new_tail.len() < self.hard_confirmations + 1 {
                    match new_tail
                        .front()
                        .unwrap()
                        .number
                        .checked_sub(1)
                        .and_then(|predecessor_number| tail.block_by_number(predecessor_number))
                    {
                        Some(predecessor) => new_tail.push_front(*predecessor),
                        None => break,
                    }
                }

                while new_tail.len() > self.hard_confirmations + 1 {
                    new_tail.pop_front();
                }

                trace!(elapsed = ?start.elapsed(), "update took");

                Ok(ChainStatus::Good {
                    tail: Tail::new(new_tail.into())?,
                })
            }
        }
    }
}

pub struct FinalityTagChainStatusUpdater {
    pub name: ChainName,
    pub rpc: Arc<dyn BlockRpc>,
}

fn block_cache(rpc: &dyn BlockRpc, range: RangeInclusive<u64>) -> Result<HashMap<Bytes32, Block>> {
    if range.is_empty() {
        return Ok(Default::default());
    }
    let blks = rpc.get_blocks(&range.map(BlockIdentifier::Height).collect::<Vec<_>>())?;
    Ok(blks
        .into_iter()
        .flat_map(|blk| blk.map(|blk| (blk.hash, blk)))
        .collect())
}

fn get_blocks_that_must_exist<const N: usize>(
    rpc: &dyn BlockRpc,
    identifiers: &[BlockIdentifier; N],
) -> anyhow::Result<[Block; N]> {
    rpc.get_blocks(identifiers)?
        .into_iter()
        .collect::<Result<Vec<Block>, _>>()?
        .try_into()
        .map_err(|r: Vec<Block>| {
            anyhow!(
                "expected {N} blocks ({identifiers:?}), got {}: {r:?}",
                r.len(),
            )
        })
}

impl ChainStatusUpdater for FinalityTagChainStatusUpdater {
    fn initial(&self) -> Result<ChainStatus> {
        Ok(ChainStatus::Good {
            tail: Tail::new(vec![self.rpc.get_block(BlockIdentifier::Finalized)?])?,
        })
    }

    fn update(&self, ctx: &Context, status: &ChainStatus) -> Result<ChainStatus> {
        match status {
            ChainStatus::FinalityViolated => Ok(ChainStatus::FinalityViolated),
            ChainStatus::Good { tail } => {
                let start = Instant::now();
                let (finalized, latest) = {
                    let [rpc_finalized, rpc_latest] = get_blocks_that_must_exist(
                        self.rpc.as_ref(),
                        &[BlockIdentifier::Finalized, BlockIdentifier::Latest],
                    )?;
                    if rpc_finalized.number <= rpc_latest.number {
                        (rpc_finalized, rpc_latest)
                    } else {
                        tracing::warn!(
                            chain=%self.name,
                            ?tail,
                            ?rpc_finalized,
                            ?rpc_latest,
                            "received finalized block with higher block number than latest block, ignoring latest block"
                        );
                        (rpc_finalized, rpc_finalized)
                    }
                };
                let cache = match block_cache(self.rpc.as_ref(), tail.tip().number..=latest.number)
                {
                    Ok(cache) => cache,
                    Err(err) => {
                        warn!(chain=%self.name, %err, "failed to build block cache; do you need to adjust your batching limits?");
                        Default::default()
                    }
                };
                let mut new_tail = VecDeque::from([latest]);
                loop {
                    if ctx.is_done() {
                        bail!("update aborted due to shutdown signal")
                    }
                    let new_tail_head = new_tail.front().unwrap();
                    if tail.contains(new_tail_head) {
                        break;
                    } else if new_tail_head.number == tail.stable_tip().number {
                        tracing::error!(chain=%self.name, ?tail, ?new_tail, "finality violated: finality tag");
                        return Ok(ChainStatus::FinalityViolated);
                    } else if new_tail_head.number == finalized.number
                        && new_tail_head.hash != finalized.hash
                    {
                        tracing::error!(chain=%self.name, ?tail, ?new_tail, ?finalized, "finality violated: conflicting finalized blocks");
                        return Ok(ChainStatus::FinalityViolated);
                    } else if new_tail_head.number < tail.stable_tip().number {
                        return Ok(ChainStatus::Good { tail: tail.clone() });
                    }

                    match cache.get(&new_tail_head.parent_hash) {
                        Some(&blk) => new_tail.push_front(blk),
                        None => {
                            trace!("cache miss for {}", new_tail_head.parent_hash);
                            new_tail.push_front(
                                self.rpc
                                    .get_block(BlockIdentifier::Hash(new_tail_head.parent_hash))?,
                            );
                        }
                    };
                }

                while new_tail.front().unwrap().number > finalized.number {
                    match new_tail
                        .front()
                        .unwrap()
                        .number
                        .checked_sub(1)
                        .and_then(|predecessor_number| tail.block_by_number(predecessor_number))
                    {
                        Some(predecessor) => new_tail.push_front(*predecessor),
                        None => break,
                    }
                }

                while new_tail.front().unwrap().number < finalized.number {
                    new_tail.pop_front();
                }

                let new_tail = Tail::new(new_tail.into())?;
                trace!(
                    elapsed = ?start.elapsed(),
                    "updated tail {} for chain {} with size {}",
                    new_tail,
                    self.name,
                    new_tail.tip().number-new_tail.stable_tip().number+1,
                );
                Ok(ChainStatus::Good { tail: new_tail })
            }
        }
    }
}

#[derive(Debug)]
pub struct ChainStatusWorker {
    pub chain_name: ChainName,
    latest_chain_status: Arc<RwLock<Option<ChainStatus>>>,
}

impl ChainStatusWorker {
    pub fn spawn(
        ctx: &Arc<Context>,
        chain_name: ChainName,
        poll_interval: Duration,
        chain_status_updater: Box<dyn ChainStatusUpdater + Send>,
        chain_metrics: Box<dyn ChainMetrics + Send>,
    ) -> (Self, ShutdownHandle) {
        let latest_chain_status = Arc::new(RwLock::new(None));
        let worker_name = format!("ChainStatusWorker({})", chain_name);

        let handle = ctx.spawn(worker_name, {
            let latest_chain_status = Arc::clone(&latest_chain_status);

            move |ctx, worker_name| -> Result<()> {
                let update_metrics = move |chain_status: &ChainStatus| {
                    if let Some(latest_block) = chain_status.tip() {
                        chain_metrics.set_latest_block_number(latest_block.number);
                    }
                    if let Some(finalized_block) = chain_status.stable_tip() {
                        chain_metrics.set_finalized_block_number(finalized_block.number);
                    }
                };

                let mut chain_status = match chain_status_updater.initial() {
                    Ok(chain_status) => {
                        update_metrics(&chain_status);
                        *latest_chain_status.write().unwrap() = Some(chain_status.clone());
                        chain_status
                    }
                    Err(err) => {
                        error!("{}: initialization failed with error {}", worker_name, err);
                        return Err(err);
                    }
                };
                info!(
                    "{}: initialization finished, starting update loop",
                    worker_name
                );
                let worker_name = worker_name.to_owned();
                let mut consecutive_failures = 0;
                ctx.repeat(poll_interval, move |ctx| {
                    match chain_status_updater.update(ctx, &chain_status) {
                        Ok(new_chain_status) => {
                            update_metrics(&new_chain_status);
                            *latest_chain_status.write().unwrap() = Some(new_chain_status.clone());
                            chain_status = new_chain_status;
                            consecutive_failures = 0;
                            Ok(())
                        }
                        Err(err) => {
                            consecutive_failures += 1;
                            warn!(
                                consecutive_failures,
                                "{worker_name}: update failed with error {}", err
                            );
                            if consecutive_failures
                                > crate::config::CHAIN_STATUS_WORKER_UPDATE_MAX_CONSECUTIVE_FAILURES
                            {
                                error!(
                                    consecutive_failures,
                                    "{worker_name}: update failed with error {}", err
                                );
                                Err(err)
                            } else {
                                Ok(())
                            }
                        }
                    }
                })
            }
        });

        (
            Self {
                chain_name,
                latest_chain_status,
            },
            handle,
        )
    }

    pub fn latest_chain_status(&self) -> Option<ChainStatus> {
        self.latest_chain_status.read().unwrap().to_owned()
    }
}
