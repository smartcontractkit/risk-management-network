use crate::{
    afn_contract::{AFNInterface, TaggedRoot},
    common::{ChainName, LaneId},
    config::ChainConfig,
    curse_beacon::CurseBeacon,
    inflight_root_cache::InflightRootCache,
    key_types::SecretKey,
    lane_bless_status::LaneBlessStatusWorker,
    metrics::ChainMetrics,
    permutation::DELTA_STAGE,
    worker,
};
use anyhow::Result;
use minieth::{rpc::Rpc, tx_sender::TransactionSender};
use std::{
    cmp::{min, Reverse},
    collections::{BinaryHeap, HashMap, HashSet},
    sync::Arc,
    time::Instant,
};
use tracing::info;

use super::{afn_voting_manager::VotingMode, worker::ShutdownHandle};

pub struct VoteToBlessWorker {
    pub chain_name: ChainName,
}

#[derive(Debug, Clone, Default)]
struct VoteQueue {
    queued_tagged_roots: BinaryHeap<Reverse<(Instant, TaggedRoot)>>,
    queued_tagged_roots_set: HashSet<TaggedRoot>,
}

impl VoteQueue {
    fn push(&mut self, send_time: Instant, tagged_root: TaggedRoot) {
        if !self.queued_tagged_roots_set.contains(&tagged_root) {
            self.queued_tagged_roots
                .push(Reverse((send_time, tagged_root)));
            self.queued_tagged_roots_set.insert(tagged_root);
        }
    }

    fn peek(&mut self) -> Option<(Instant, TaggedRoot)> {
        if let Some(&Reverse((send_time, tagged_root))) = self.queued_tagged_roots.peek() {
            return Some((send_time, tagged_root));
        }
        None
    }

    fn pop(&mut self) -> Option<TaggedRoot> {
        if let Some((send_time, tagged_root)) = self.peek() {
            if Instant::now() >= send_time {
                self.queued_tagged_roots_set.remove(&tagged_root);
                self.queued_tagged_roots.pop();
                return Some(tagged_root);
            }
        }
        None
    }
}

impl VoteToBlessWorker {
    pub fn spawn(
        ctx: &Arc<worker::Context>,
        rpc: Arc<Rpc>,
        config: &ChainConfig,
        onchain_config: crate::afn_contract::OnchainConfig,
        lane_bless_status_workers: HashMap<LaneId, Arc<LaneBlessStatusWorker>>,
        poll_interval: std::time::Duration,
        key: SecretKey,
        curse_beacon: Arc<CurseBeacon>,
        mode: VotingMode,
        chain_metrics: Box<dyn ChainMetrics + Send>,
    ) -> Result<(Self, ShutdownHandle)> {
        let worker_name = format!("VoteToBlessWorker({},{})", config.name, config.afn_contract);
        let handle = ctx.spawn(worker_name, {
            let config = config.clone();
            move |ctx, worker_name| -> Result<()> {
                let bless_vote_addr = key.address();

                let afn_contract =
                    AFNInterface::create_from_chain_config(Arc::clone(&rpc), &config)?;
                let mut tx_sender = TransactionSender::new(
                    config.name.chain_id(),
                    config.bless_fee_config,
                    Arc::clone(&rpc),
                    Arc::new(key.local_signer()),
                );

                let mut inflight = InflightRootCache::new(config.inflight_time.into());
                let mut queue = VoteQueue::default();

                let sorted_lane_bless_status_workers = {
                    let mut lane_bless_status_workers =
                        lane_bless_status_workers.into_iter().collect::<Vec<_>>();
                    lane_bless_status_workers.sort_by_key(|(lane_id, _)| lane_id.clone());
                    lane_bless_status_workers
                };

                while !ctx.is_done() && !curse_beacon.is_cursed() {
                    let votable = Self::get_roots_to_bless(&sorted_lane_bless_status_workers);
                    if let Some(delay) =
                        crate::permutation::delay(bless_vote_addr, &onchain_config, mode)
                    {
                        if !votable.is_empty() {
                            tracing::info!(
                                "{worker_name}: need to wait {delay:?} to vote to bless {} roots...",
                                votable.len()
                            );
                        }
                        let send_time = Instant::now() + delay;
                        votable
                            .iter()
                            .filter(|&&tagged_root| !inflight.contains(tagged_root))
                            .for_each(|&tagged_root| {
                                queue.push(send_time, tagged_root)
                            });
                    } else {
                        tracing::error!(
                            "{worker_name}: no vote to bless delay derivable for voter {bless_vote_addr:?}; \
                            is the voter part of the onchain config?"
                        );
                    }

                    while !ctx.is_done()
                        && !curse_beacon.is_cursed()
                    {
                        let mut batch = Vec::new();
                        while batch.len() < config.max_tagged_roots_per_vote_to_bless {
                            if let Some(tagged_root) = queue.pop() {
                                if votable.contains(&tagged_root) && !inflight.contains(tagged_root)
                                {
                                    batch.push(tagged_root);
                                }
                            } else {
                                break;
                            }
                        }

                        if batch.is_empty() {
                            break;
                        }

                        let txid = match mode {
                            VotingMode::Active => Some(
                                afn_contract.send_vote_to_bless(&mut tx_sender, batch.clone())?,
                            ),
                            VotingMode::DryRun | VotingMode::Passive => None,
                        };

                        if txid.is_some() {
                            chain_metrics.inc_bless_tx_counter();
                        }
                        info!(
                            ?txid,
                            "{worker_name}: voted to bless {} tagged roots: {:?}",
                            batch.len(),
                            batch
                        );
                        batch
                            .iter()
                            .for_each(|tagged_root| inflight.add(*tagged_root));
                    }

                    let until_next_send_time = queue
                        .peek()
                        .map(|(send_time, _)| send_time - Instant::now())
                        .unwrap_or(DELTA_STAGE);
                    std::thread::sleep(min(poll_interval, until_next_send_time));
                }
                Ok(())
            }
        });
        Ok((
            Self {
                chain_name: config.name,
            },
            handle,
        ))
    }

    fn get_roots_to_bless(
        lane_bless_status_workers: &[(LaneId, Arc<LaneBlessStatusWorker>)],
    ) -> Vec<TaggedRoot> {
        lane_bless_status_workers
            .iter()
            .filter_map(|(_, worker)| worker.lane_bless_status.lock().unwrap().clone())
            .flat_map(|status| status.verified_tagged_roots)
            .collect()
    }
}
