use crate::chain_status::{ChainStatus, Tail};
use crate::common::ChainName;
use crate::metrics::ChainStatusMetricsHandle;
use crate::worker::{Context, ShutdownHandle};
use anyhow::{anyhow, bail, ensure, Context as _, Result};
use minieth::bytes::Bytes32;
use minieth::rpc::{Block, BlockIdentifier};
use minieth::rpc::{BlockRpc, PolkadotBlockRpc};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use tracing::{trace, warn};

pub trait ChainStatusUpdater {
    fn initial(&self) -> Result<ChainStatus>;
    fn update(&self, ctx: &Context, status: &ChainStatus) -> Result<ChainStatus>;
}

pub struct ConfirmationDepthChainStatusUpdater {
    pub soft_confirmations: usize,
    pub hard_confirmations: usize,
    pub rpc: Arc<dyn BlockRpc>,
}

impl GetLatestAndStableBlocks for ConfirmationDepthChainStatusUpdater {
    fn get_latest_and_stable_blocks(&self) -> Result<LatestAndStableBlocks> {
        let latest = self
            .rpc
            .get_block(BlockIdentifier::Latest)
            .context("failed to get latest block")?;
        let soft_confirmed_number = latest
            .number
            .checked_sub(self.soft_confirmations as u64)
            .ok_or_else(|| {
                anyhow!(
                    "no soft confirmed block available yet (latest.number={}, soft_confirmations={})",
                    latest.number,
                    self.soft_confirmations
                )
            })?;
        let hard_confirmed_number =
            soft_confirmed_number.checked_sub(self.hard_confirmations as u64)
            .ok_or_else(|| {
                anyhow!(
                    "no hard confirmed block available yet (soft_confirmed.number={}, hard_confirmations={})",
                    soft_confirmed_number,
                    self.hard_confirmations
                )
            })?;

        let [hard_confirmed, soft_confirmed] = get_blocks_that_must_exist(
            self.rpc.as_ref(),
            &[
                BlockIdentifier::Height(hard_confirmed_number),
                BlockIdentifier::Height(soft_confirmed_number),
            ],
        )
        .context("failed to get hard and soft confirmed blocks")?;
        Ok(LatestAndStableBlocks {
            latest: soft_confirmed,
            stable: hard_confirmed,
        })
    }

    fn get_block_rpc(&self) -> &dyn BlockRpc {
        self.rpc.as_ref()
    }
}

pub struct FinalityTagChainStatusUpdater {
    pub rpc: Arc<dyn BlockRpc>,
}

pub struct PolkadotGetFinalizedHeadChainStatusUpdater {
    pub rpc: Arc<dyn BlockRpc>,
    pub polkadot_rpc: Arc<dyn PolkadotBlockRpc>,
}

fn get_block_and_populate_cache_if_needed(
    cache: &mut HashMap<Bytes32, Block>,
    rpc: &dyn BlockRpc,
    min_block_number: u64,
    (block_number, block_hash): (u64, Bytes32),
) -> Result<Block> {
    if let Some(&blk) = cache.get(&block_hash) {
        return Ok(blk);
    }

    trace!(block_number, %block_hash, "cache miss");

    let mut blocks_to_request = vec![BlockIdentifier::Hash(block_hash)];

    let min_block_number_to_cache = std::cmp::max(
        min_block_number,
        block_number.saturating_sub(BATCH_FETCH_BLOCKS_LIMIT.saturating_sub(1)),
    );
    for block_number in min_block_number_to_cache..block_number {
        blocks_to_request.push(BlockIdentifier::Height(block_number));
    }

    ensure!(
        blocks_to_request.len() <= BATCH_FETCH_BLOCKS_LIMIT as usize,
        "assumption violation: too many blocks to request: {} > {}",
        blocks_to_request.len(),
        BATCH_FETCH_BLOCKS_LIMIT
    );

    let rpc_blocks = match rpc.get_blocks(&blocks_to_request) {
        Ok(batch_of_blocks) => batch_of_blocks,
        Err(err) => {
            warn!(
                ?err,
                "failed to get batch of blocks, resorting to sequential block fetching! \
                usually due to misconfigured rpc settings: batch-response-max-size, batch-request-limit"
            );
            vec![rpc.get_block(BlockIdentifier::Hash(block_hash))]
        }
    };

    let mut rpc_blocks = VecDeque::from(rpc_blocks);

    let first_rpc_block = rpc_blocks
        .pop_front()
        .context("assumption violation: not all batch calls were responded to, though outer result was Ok")?
        .with_context(|| format!("expected block with hash {}", block_hash))?;

    ensure!(
        (first_rpc_block.hash, first_rpc_block.number) == (block_hash, block_number),
        "expected block with number {} and hash {}, got block {:?}",
        block_number,
        block_hash,
        first_rpc_block
    );

    cache.clear();
    cache.extend::<HashMap<Bytes32, Block>>(
        rpc_blocks
            .drain(..)
            .flatten()
            .map(|block| (block.hash, block))
            .collect(),
    );

    Ok(first_rpc_block)
}

const BATCH_FETCH_BLOCKS_LIMIT: u64 = 100 - 1;

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

struct LatestAndStableBlocks {
    latest: Block,
    stable: Block,
}

trait GetLatestAndStableBlocks {
    fn get_latest_and_stable_blocks(&self) -> Result<LatestAndStableBlocks>;
    fn get_block_rpc(&self) -> &dyn BlockRpc;
}

impl GetLatestAndStableBlocks for FinalityTagChainStatusUpdater {
    fn get_latest_and_stable_blocks(&self) -> Result<LatestAndStableBlocks> {
        let [finalized, latest] = get_blocks_that_must_exist(
            self.rpc.as_ref(),
            &[BlockIdentifier::Finalized, BlockIdentifier::Latest],
        )
        .context("failed to get finalized and latest blocks")?;
        Ok(LatestAndStableBlocks {
            latest,
            stable: finalized,
        })
    }

    fn get_block_rpc(&self) -> &dyn BlockRpc {
        self.rpc.as_ref()
    }
}

impl GetLatestAndStableBlocks for PolkadotGetFinalizedHeadChainStatusUpdater {
    fn get_latest_and_stable_blocks(&self) -> Result<LatestAndStableBlocks> {
        let polkadot_finalized_number = {
            let polkadot_finalized_head_hash = self
                .polkadot_rpc
                .polkadot_get_finalized_head()
                .context("failed to get Polkadot finalized head hash via chain_getFinalizedHead")?;
            let polkadot_block = self
                .polkadot_rpc
                .polkadot_get_block(polkadot_finalized_head_hash)
                .context("failed to get Polkadot signed block via chain_getBlock")?;
            polkadot_block.header.number
        };

        let [finalized, latest] = get_blocks_that_must_exist(
            self.rpc.as_ref(),
            &[
                BlockIdentifier::Height(polkadot_finalized_number),
                BlockIdentifier::Latest,
            ],
        )
        .context("failed to get Polkadot finalized and latest blocks")?;

        Ok(LatestAndStableBlocks {
            latest,
            stable: finalized,
        })
    }

    fn get_block_rpc(&self) -> &dyn BlockRpc {
        self.rpc.as_ref()
    }
}

impl<T: GetLatestAndStableBlocks> ChainStatusUpdater for T {
    fn initial(&self) -> Result<ChainStatus> {
        let LatestAndStableBlocks { stable, .. } = self.get_latest_and_stable_blocks()?;
        Ok(ChainStatus::Good {
            tail: Tail::new(vec![stable])?,
        })
    }

    fn update(&self, ctx: &Context, status: &ChainStatus) -> Result<ChainStatus> {
        match status {
            ChainStatus::FinalityViolated => Ok(ChainStatus::FinalityViolated),
            ChainStatus::Good { tail } => {
                let start = Instant::now();
                let LatestAndStableBlocks { mut latest, stable } =
                    self.get_latest_and_stable_blocks()?;
                if stable.number > latest.number {
                    warn!(
                        ?stable,
                        ?latest,
                        "received stable block with higher block number than latest block, ignoring latest block"
                    );
                    latest = stable;
                }

                let mut new_tail = VecDeque::from([latest]);

                let mut cache: HashMap<Bytes32, Block> = HashMap::new();

                loop {
                    if ctx.is_done() {
                        bail!("update aborted due to shutdown signal")
                    }
                    let new_tail_head = new_tail.front().unwrap();
                    if tail.contains(new_tail_head) {
                        break;
                    } else if new_tail_head.number == tail.stable_tip().number {
                        tracing::error!(
                            ?tail,
                            ?new_tail,
                            "finality violated: latest block leads to block at the same number but with different hash \
                            from the prior stable tip"
                        );
                        return Ok(ChainStatus::FinalityViolated);
                    } else if new_tail_head.number == stable.number
                        && new_tail_head.hash != stable.hash
                    {
                        tracing::error!(
                            ?tail,
                            ?new_tail,
                            ?stable,
                            "finality violated: latest block leads to block at the same number but with different hash \
                            from the stable block queried at the beginning of the update"
                        );
                        return Ok(ChainStatus::FinalityViolated);
                    } else if new_tail_head.number < tail.stable_tip().number {
                        warn!(
                            ?tail,
                            ?new_tail,
                            "new tail rewound past tail's stable tip; ignoring new tail"
                        );
                        return Ok(ChainStatus::Good { tail: tail.clone() });
                    }

                    let (parent_block_number, parent_block_hash) = (
                        new_tail_head
                            .number
                            .checked_sub(1)
                            .context("cannot go beyond genesis block")?,
                        new_tail_head.parent_hash,
                    );
                    let parent_block = match tail.block_by_number(parent_block_number) {
                        Some(tails_block)
                            if (tails_block.number, tails_block.hash)
                                == (parent_block_number, parent_block_hash) =>
                        {
                            *tails_block
                        }
                        _ => {
                            let min_block_number_to_populate_cache_with =
                                if parent_block_number > tail.tip().number {
                                    tail.tip().number
                                } else {
                                    tail.stable_tip().number
                                };

                            get_block_and_populate_cache_if_needed(
                                &mut cache,
                                self.get_block_rpc(),
                                min_block_number_to_populate_cache_with,
                                (parent_block_number, parent_block_hash),
                            )?
                        }
                    };
                    new_tail.push_front(parent_block);
                }

                while new_tail.front().unwrap().number > stable.number {
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

                while new_tail.front().unwrap().number < stable.number {
                    new_tail.pop_front();
                }

                let new_tail = Tail::new(new_tail.into())?;
                trace!(
                    elapsed = ?start.elapsed(),
                    "updated tail {} with size {}",
                    new_tail,
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
        chain_status_metrics_handle: ChainStatusMetricsHandle,
    ) -> (Self, ShutdownHandle) {
        let latest_chain_status = Arc::new(RwLock::new(None));
        let worker_name = format!("ChainStatusWorker({})", chain_name);

        let handle = ctx.spawn(worker_name, {
            let latest_chain_status = Arc::clone(&latest_chain_status);

            move |ctx, worker_name| -> Result<()> {
                let worker_name = worker_name.to_owned();

                let update_metrics = move |chain_status: &ChainStatus| {
                    if let Some(latest_block) = chain_status.tip() {
                        chain_status_metrics_handle
                            .latest_block_number
                            .set(latest_block.number as f64);

                        chain_status_metrics_handle
                            .latest_block_timestamp
                            .set(latest_block.timestamp as f64)
                    }

                    if let Some(finalized_block) = chain_status.stable_tip() {
                        chain_status_metrics_handle
                            .finalized_block_number
                            .set(finalized_block.number as f64);

                        chain_status_metrics_handle
                            .finalized_block_timestamp
                            .set(finalized_block.timestamp as f64);
                    }

                    chain_status_metrics_handle.finality_violated.set(
                        if matches!(chain_status, ChainStatus::FinalityViolated) {
                            1f64
                        } else {
                            0f64
                        },
                    );
                };

                let span = tracing::error_span!("chain status", chain=%chain_name);
                let _enter = span.enter();

                ctx.repeat(worker_name.clone(), poll_interval, move |ctx| {
                    let new_chain_status = match latest_chain_status.read().unwrap().as_ref() {
                        None => chain_status_updater.initial(),
                        Some(chain_status) => chain_status_updater.update(ctx, chain_status),
                    }?;
                    update_metrics(&new_chain_status);
                    *latest_chain_status.write().unwrap() = Some(new_chain_status);
                    Ok(())
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
}

impl ChainStatusProvider for ChainStatusWorker {
    fn latest_chain_status(&self) -> Option<ChainStatus> {
        self.latest_chain_status.read().unwrap().to_owned()
    }
}

pub trait ChainStatusProvider: Sync + Send {
    fn latest_chain_status(&self) -> Option<ChainStatus>;
}
