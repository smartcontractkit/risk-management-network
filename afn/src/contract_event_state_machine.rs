use crate::{
    cached_rpc::CachedRpc,
    chain_status::{age, ChainStatus},
    chain_status_worker::ChainStatusWorker,
    common::{ChainName, DecodeLog, LogTopic},
    config::{ChainStability, LaneType},
    evm2evm_onramp_v1_0::CcipSendRequestedEventV1_0,
    evm2evm_onramp_v1_2::CcipSendRequestedEventV1_2,
    onramp_traits::OnRampReader,
    worker::{self, ShutdownHandle},
};
use anyhow::{anyhow, bail, Context, Result};
use bit_set::BitSet;
use minieth::{
    bytes::{Address, Bytes32},
    rpc::{EVMLog, Rpc},
};
use std::{
    sync::{Arc, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard},
    time::{Duration, Instant},
};
use tracing::{trace, warn};

pub trait ContractEventFilter: Send + Sync + Clone {
    type ContractEventUnion: Sized;
    fn log_topics(&self) -> Vec<Bytes32>;
    fn decode_log(&self, log: EVMLog) -> Result<Self::ContractEventUnion>;
}

pub trait State: Clone {
    type ContractEventUnion: Sized;
    type ContractEventFilter: ContractEventFilter<ContractEventUnion = Self::ContractEventUnion>;
    fn update_with_event(
        &mut self,
        block_number: u64,
        block_hash: Bytes32,
        address: Address,
        filter: &Self::ContractEventFilter,
        ev: Self::ContractEventUnion,
    ) -> Result<()>;
}

#[derive(Clone, Debug)]
enum UpdateProgress {
    NextStartBlockNumber(u64),
    Corrupted,
}

pub enum ContractStateMachineUpdateResult {
    Complete,
    Partial { block_number: u64 },
}

pub struct ContractEventStateMachine<S: State> {
    max_fresh_block_age: Duration,
    addresses: Vec<Address>,
    update_progress: Arc<Mutex<UpdateProgress>>,
    timestamp: Arc<Mutex<Option<u64>>>,
    state: Arc<RwLock<S>>,
    contract_event_filter: S::ContractEventFilter,
}

impl<S: State> Clone for ContractEventStateMachine<S> {
    fn clone(&self) -> Self {
        let update_progress = self.update_progress.lock().unwrap();

        Self {
            max_fresh_block_age: self.max_fresh_block_age,
            addresses: self.addresses.clone(),
            update_progress: Arc::new(Mutex::new(update_progress.clone())),
            timestamp: Arc::new(Mutex::new(*self.timestamp.lock().unwrap())),
            state: Arc::new(RwLock::new(self.state.write().unwrap().clone())),
            contract_event_filter: self.contract_event_filter.clone(),
        }
    }
}

impl<S: State> ContractEventStateMachine<S> {
    pub fn new(
        max_fresh_block_age: Duration,
        start_block_number: u64,
        addresses: Vec<Address>,
        initial_state: S,
        contract_event_filter: S::ContractEventFilter,
    ) -> Self {
        Self {
            max_fresh_block_age,
            addresses,
            update_progress: Arc::new(Mutex::new(UpdateProgress::NextStartBlockNumber(
                start_block_number,
            ))),
            timestamp: Arc::new(Mutex::new(None)),
            state: Arc::new(RwLock::new(initial_state)),
            contract_event_filter,
        }
    }

    fn fork_from(&self, other: &Self) -> Result<()> {
        if other.addresses != self.addresses {
            bail!("addresses mismatch");
        }

        let mut update_progress = self.update_progress.lock().unwrap();
        let mut state = self.state.write().unwrap();
        let mut timestamp = self.timestamp.lock().unwrap();

        let other_update_progress = other.update_progress.lock().unwrap();
        let other_state = other.state.read().unwrap();
        let other_timestamp = other.timestamp.lock().unwrap();

        update_progress.clone_from(&other_update_progress);
        state.clone_from(&other_state);
        timestamp.clone_from(&other_timestamp);

        Ok(())
    }

    pub fn update(
        &self,
        cached_rpc: &CachedRpc,
        chain_status: &ChainStatus,
        finality: Finality,
    ) -> Result<ContractStateMachineUpdateResult> {
        let mut update_progress = self.update_progress.lock().unwrap();

        let start_block_number = match *update_progress {
            UpdateProgress::NextStartBlockNumber(i) => i,
            UpdateProgress::Corrupted => return Err(anyhow!("state machine broke down")),
        };
        let tail = match chain_status {
            ChainStatus::Good { tail } => tail,
            ChainStatus::FinalityViolated => {
                *update_progress = UpdateProgress::Corrupted;
                return Err(anyhow!("finality violated"));
            }
        };
        let tip = tail.tip();
        let max_end_block_number = match finality {
            Finality::Stable => tail.stable_tip().number,
            Finality::Unstable => tip.number,
        };

        if max_end_block_number < start_block_number {
            *self.timestamp.lock().unwrap() = Some(tip.timestamp);
            return Ok(ContractStateMachineUpdateResult::Complete);
        }

        let partial_logs = cached_rpc.get_partial_logs(
            start_block_number,
            max_end_block_number,
            match finality {
                Finality::Stable => true,
                Finality::Unstable => false,
            },
        )?;

        let end_block_number = partial_logs.to_block;
        let logs = partial_logs.logs;

        {
            let mut state = self.state.write().unwrap();

            for log in logs {
                let (block_number, block_hash, address) =
                    (log.block_number, log.block_hash, log.address);
                if matches!(finality, Finality::Unstable) {
                    trace!("verifying {} in {chain_status:?}", log.block_hash);
                    let same_block_from_tail = chain_status.block_by_number(log.block_number).ok_or_else(|| {
                        anyhow!("we have a bug: block number found in logs {block_number} not in chain status {:?}", chain_status)
                    })?;
                    if same_block_from_tail.hash != block_hash {
                        *update_progress = UpdateProgress::Corrupted;
                        return Err(anyhow!(
                            "mismatch between tail ({:?}) and log ({:?})",
                            same_block_from_tail,
                            log
                        ));
                    }
                }
                let ev = self.contract_event_filter.decode_log(log)?;
                if let Err(e) = state.update_with_event(
                    block_number,
                    block_hash,
                    address,
                    &self.contract_event_filter,
                    ev,
                ) {
                    *update_progress = UpdateProgress::Corrupted;
                    return Err(e);
                }
            }

            *update_progress = UpdateProgress::NextStartBlockNumber(end_block_number + 1);
        }

        Ok(if end_block_number == max_end_block_number {
            *self.timestamp.lock().unwrap() = Some(tip.timestamp);
            ContractStateMachineUpdateResult::Complete
        } else {
            ContractStateMachineUpdateResult::Partial {
                block_number: end_block_number,
            }
        })
    }

    pub fn state_mut(&self) -> RwLockWriteGuard<S> {
        self.state.write().unwrap()
    }

    pub fn state_read(&self) -> RwLockReadGuard<S> {
        self.state.read().unwrap()
    }

    pub fn synced_state_read(&self) -> (RwLockReadGuard<S>, bool) {
        let is_synced = {
            let timestamp = *self.timestamp.lock().unwrap();
            if let Some(latest_block_timestamp) = timestamp {
                age(latest_block_timestamp) <= self.max_fresh_block_age
            } else {
                false
            }
        };
        (self.state_read(), is_synced)
    }
}

pub struct UnstableContractEventStateMachineWorker<S: State> {
    stable_cesm: Arc<ContractEventStateMachine<S>>,
    unstable_cesm: Arc<ContractEventStateMachine<S>>,
}

#[derive(Clone, Copy, Debug)]
pub enum Finality {
    Stable,
    Unstable,
}

fn keep_updating_until_complete_or_shutdown_request<S: State + Send + 'static>(
    ctx: &worker::Context,
    chain_status: &ChainStatus,
    rpc: &CachedRpc,
    cesm: &ContractEventStateMachine<S>,
    finality: Finality,
) -> Result<()> {
    while !ctx.is_done() {
        match cesm
            .update(rpc, chain_status, finality)
            .context("cesm update failure")?
        {
            ContractStateMachineUpdateResult::Complete => {
                break;
            }
            ContractStateMachineUpdateResult::Partial { block_number: _ } => {}
        }
    }
    Ok(())
}

impl<S: State + Send + Sync + 'static> UnstableContractEventStateMachineWorker<S> {
    pub fn spawn(
        ctx: &Arc<worker::Context>,
        worker_name: &str,
        poll_interval: std::time::Duration,
        cesm: ContractEventStateMachine<S>,
        chain: ChainName,
        rpc: Arc<Rpc>,
        chain_status_worker: std::sync::Arc<ChainStatusWorker>,
        chain_stability: ChainStability,
    ) -> (Self, ShutdownHandle) {
        let log_topics = cesm.contract_event_filter.log_topics();
        let stable_cesm = Arc::new(cesm);
        let unstable_cesm = Arc::new(stable_cesm.as_ref().clone());
        let handle = ctx.spawn(worker_name.to_string(), {
            let stable_cesm = Arc::clone(&stable_cesm);
            let unstable_cesm = Arc::clone(&unstable_cesm);
            move |ctx, worker_name| -> Result<()> {
                let cached_rpc = CachedRpc::new(
                    chain,
                    &stable_cesm.addresses,
                    log_topics,
                    rpc,
                    chain_stability,
                    worker_name,
                )?;
                let mut reported_initial_sync = false;
                let worker_name = worker_name.to_owned();
                ctx.repeat(worker_name.clone(), poll_interval, move |ctx| {
                    if let Some(chain_status) = chain_status_worker.latest_chain_status() {
                        let start_sync = Instant::now();
                        keep_updating_until_complete_or_shutdown_request(
                            ctx,
                            &chain_status,
                            &cached_rpc,
                            &stable_cesm,
                            Finality::Stable,
                        )
                        .with_context(|| format!("{worker_name} stable update"))?;
                        if !reported_initial_sync {
                            tracing::info!(
                                "initial sync of {worker_name} in {:?}",
                                start_sync.elapsed()
                            );
                            reported_initial_sync = true;
                        }

                        let new_unstable_cesm = (*stable_cesm).clone();

                        match keep_updating_until_complete_or_shutdown_request(
                            ctx,
                            &chain_status,
                            &cached_rpc,
                            &new_unstable_cesm,
                            Finality::Unstable,
                        ) {
                            Ok(_) => {
                                unstable_cesm.fork_from(&new_unstable_cesm).unwrap();
                            }
                            Err(e) => {
                                return Err(e)
                                    .with_context(|| format!("{worker_name} unstable update"));
                            }
                        }
                    } else {
                        warn!("{worker_name}: chain status not initialized yet");
                    }
                    Ok(())
                })
            }
        });
        (
            Self {
                stable_cesm,
                unstable_cesm,
            },
            handle,
        )
    }

    pub fn stable_state_mut(&self) -> RwLockWriteGuard<S> {
        self.stable_cesm.state_mut()
    }

    pub fn stable_state_read(&self) -> RwLockReadGuard<S> {
        self.stable_cesm.state_read()
    }

    pub fn synced_stable_state_read(&self) -> (RwLockReadGuard<S>, bool) {
        self.stable_cesm.synced_state_read()
    }

    pub fn unstable_state_read(&self) -> RwLockReadGuard<S> {
        self.unstable_cesm.state_read()
    }

    pub fn unstable_and_stable_state_read(&self) -> (RwLockReadGuard<S>, RwLockReadGuard<S>) {
        (
            self.unstable_cesm.state_read(),
            self.stable_cesm.state_read(),
        )
    }

    pub fn synced_unstable_and_stable_state_read(
        &self,
    ) -> ((RwLockReadGuard<S>, bool), (RwLockReadGuard<S>, bool)) {
        (
            self.unstable_cesm.synced_state_read(),
            self.stable_cesm.synced_state_read(),
        )
    }
}

#[derive(Debug, Clone)]
pub enum CcipSendRequestedEventUnion {
    Evm2EvmV1_0(CcipSendRequestedEventV1_0),
    Evm2EvmV1_2(CcipSendRequestedEventV1_2),
}

#[derive(Clone)]
pub struct VersionedCcipSendRequestFilter {
    pub lane_type: crate::config::LaneType,
}

impl ContractEventFilter for VersionedCcipSendRequestFilter {
    type ContractEventUnion = CcipSendRequestedEventUnion;
    fn log_topics(&self) -> Vec<Bytes32> {
        match self.lane_type {
            LaneType::Evm2EvmV1_0 => {
                vec![CcipSendRequestedEventV1_0::log_topic()]
            }
            LaneType::Evm2EvmV1_2 => {
                vec![CcipSendRequestedEventV1_2::log_topic()]
            }
        }
    }
    fn decode_log(&self, log: EVMLog) -> Result<Self::ContractEventUnion> {
        let &topic = log
            .topics
            .first()
            .ok_or(anyhow!("cannot decode log with empty topic"))?;
        if matches!(self.lane_type, LaneType::Evm2EvmV1_0)
            && topic == CcipSendRequestedEventV1_0::log_topic()
        {
            Ok(CcipSendRequestedEventUnion::Evm2EvmV1_0(
                CcipSendRequestedEventV1_0::decode_log(log)?,
            ))
        } else if matches!(self.lane_type, LaneType::Evm2EvmV1_2)
            && topic == CcipSendRequestedEventV1_2::log_topic()
        {
            Ok(CcipSendRequestedEventUnion::Evm2EvmV1_2(
                CcipSendRequestedEventV1_2::decode_log(log)?,
            ))
        } else {
            Err(anyhow!(
                "unknown topic {:?} for lane type {:?}",
                topic,
                self.lane_type
            ))
        }
    }
}

#[derive(Clone)]
pub struct SourceBlessState {
    pub message_metadata: crate::hashable::MessageMetadata,
    pub seq_nrs_and_message_hashes: std::collections::VecDeque<(u64, Bytes32)>,
}

impl State for SourceBlessState {
    type ContractEventUnion = CcipSendRequestedEventUnion;
    type ContractEventFilter = VersionedCcipSendRequestFilter;
    fn update_with_event(
        &mut self,
        _block_number: u64,
        _block_hash: Bytes32,
        _address: Address,
        filter: &Self::ContractEventFilter,
        ev: CcipSendRequestedEventUnion,
    ) -> Result<()> {
        use crate::hashable::Hashable;
        let (seq_nr, msg_hash, msg_id) = match (filter.lane_type, ev) {
            (LaneType::Evm2EvmV1_0, CcipSendRequestedEventUnion::Evm2EvmV1_0(ev)) => {
                let msg = ev.message;
                (
                    msg.sequence_number,
                    msg.hash(&self.message_metadata),
                    msg.message_id,
                )
            }
            (LaneType::Evm2EvmV1_2, CcipSendRequestedEventUnion::Evm2EvmV1_2(ev)) => {
                let msg = ev.message;
                (
                    msg.sequence_number,
                    msg.hash(&self.message_metadata),
                    msg.message_id,
                )
            }
            (ty @ LaneType::Evm2EvmV1_0, ev @ CcipSendRequestedEventUnion::Evm2EvmV1_2(_))
            | (ty @ LaneType::Evm2EvmV1_2, ev @ CcipSendRequestedEventUnion::Evm2EvmV1_0(_)) => {
                bail!("lane type mismatch: expected message of type {ty:?} but got {ev:?}",);
            }
        };
        if msg_hash != msg_id {
            bail!("hash mismatch for seq_nr {seq_nr}! we computed: {msg_hash}, event contains: {msg_id}",);
        }

        if let Some(&(last_seq_nr, _)) = self.seq_nrs_and_message_hashes.back() {
            if last_seq_nr + 1 != seq_nr {
                bail!("last seq_nr is {last_seq_nr} and we're trying to push seq_nr {seq_nr} which is not contiguous");
            }
        } else if seq_nr != 1 {
            bail!("the first seq_nr should be 1 but we got {seq_nr}, potentially incorrect start_block_number");
        }

        self.seq_nrs_and_message_hashes
            .push_back((seq_nr, msg_hash));

        Ok(())
    }
}

impl OnRampReader for SourceBlessState {
    fn get_message_hashes(
        &self,
        interval: &crate::commit_store_common::Interval,
    ) -> Option<Vec<Bytes32>> {
        let &(initial_seq_nr, _) = self.seq_nrs_and_message_hashes.front()?;

        let start_index = (interval.min as usize).checked_sub(initial_seq_nr as usize)?;
        let end_index = start_index.checked_add(interval.max.checked_sub(interval.min)? as usize)?;

        if end_index >= self.seq_nrs_and_message_hashes.len() {
            None
        } else {
            Some(
                self.seq_nrs_and_message_hashes
                    .range(start_index..=end_index)
                    .map(|&(_, message_hash)| message_hash)
                    .collect(),
            )
        }
    }
}

impl SourceBlessState {
    pub fn try_reap_up_to(&mut self, seq_nr: u64) {
        let index = self
            .seq_nrs_and_message_hashes
            .partition_point(|&(s, _)| s <= seq_nr);
        self.seq_nrs_and_message_hashes.drain(..index);
    }

    pub fn message_hash(&self, seq_nr: u64) -> Option<&Bytes32> {
        self.seq_nrs_and_message_hashes
            .front()
            .and_then(|(min_seq_nr, _)| seq_nr.checked_sub(*min_seq_nr))
            .and_then(|idx| self.seq_nrs_and_message_hashes.get(idx as usize))
            .map(|(_, msg_hash)| msg_hash)
    }
}

pub type SourceBlessWorker = UnstableContractEventStateMachineWorker<SourceBlessState>;

#[derive(Clone, Debug)]
pub enum DestEventUnion {
    Blessed(crate::afn_contract::TaggedRootBlessedEvent),
    BlessVotesReset(crate::afn_contract::TaggedRootBlessVotesResetEvent),
    AlreadyBlessed(crate::afn_contract::AlreadyBlessedEvent),
    Voted(crate::afn_contract::VotedToBlessEvent),
    AlreadyVoted(crate::afn_contract::AlreadyVotedToBlessEvent),
    CommittedV1_0(crate::commit_store_v1_0::ReportAcceptedEventV1_0),
    CommittedV1_2(crate::commit_store_v1_2::ReportAcceptedEventV1_2),
}
#[derive(Clone)]
pub struct VersionedDestEventFilter {
    pub lane_type: crate::config::LaneType,
}

impl ContractEventFilter for VersionedDestEventFilter {
    type ContractEventUnion = DestEventUnion;
    fn log_topics(&self) -> Vec<Bytes32> {
        let non_versioned_topics = vec![
            crate::afn_contract::TaggedRootBlessedEvent::log_topic(),
            crate::afn_contract::TaggedRootBlessVotesResetEvent::log_topic(),
            crate::afn_contract::AlreadyBlessedEvent::log_topic(),
            crate::afn_contract::VotedToBlessEvent::log_topic(),
            crate::afn_contract::AlreadyVotedToBlessEvent::log_topic(),
        ];
        let versioned_topics = match self.lane_type {
            LaneType::Evm2EvmV1_0 => {
                vec![crate::commit_store_v1_0::ReportAcceptedEventV1_0::log_topic()]
            }
            LaneType::Evm2EvmV1_2 => {
                vec![crate::commit_store_v1_2::ReportAcceptedEventV1_2::log_topic()]
            }
        };
        [non_versioned_topics, versioned_topics].concat()
    }

    fn decode_log(&self, log: EVMLog) -> Result<Self::ContractEventUnion> {
        let &topic = log
            .topics
            .first()
            .ok_or(anyhow!("cannot decode log with empty topic"))?;
        if topic == crate::afn_contract::TaggedRootBlessedEvent::log_topic() {
            Ok(DestEventUnion::Blessed(
                crate::afn_contract::TaggedRootBlessedEvent::decode_log(log)?,
            ))
        } else if topic == crate::afn_contract::TaggedRootBlessVotesResetEvent::log_topic() {
            Ok(DestEventUnion::BlessVotesReset(
                crate::afn_contract::TaggedRootBlessVotesResetEvent::decode_log(log)?,
            ))
        } else if topic == crate::afn_contract::AlreadyBlessedEvent::log_topic() {
            Ok(DestEventUnion::AlreadyBlessed(
                crate::afn_contract::AlreadyBlessedEvent::decode_log(log)?,
            ))
        } else if topic == crate::afn_contract::VotedToBlessEvent::log_topic() {
            Ok(DestEventUnion::Voted(
                crate::afn_contract::VotedToBlessEvent::decode_log(log)?,
            ))
        } else if topic == crate::afn_contract::AlreadyVotedToBlessEvent::log_topic() {
            Ok(DestEventUnion::AlreadyVoted(
                crate::afn_contract::AlreadyVotedToBlessEvent::decode_log(log)?,
            ))
        } else if topic == crate::commit_store_v1_0::ReportAcceptedEventV1_0::log_topic()
            && matches!(self.lane_type, LaneType::Evm2EvmV1_0)
        {
            Ok(DestEventUnion::CommittedV1_0(
                crate::commit_store_v1_0::ReportAcceptedEventV1_0::decode_log(log)?,
            ))
        } else if topic == crate::commit_store_v1_2::ReportAcceptedEventV1_2::log_topic()
            && matches!(self.lane_type, LaneType::Evm2EvmV1_2)
        {
            Ok(DestEventUnion::CommittedV1_2(
                crate::commit_store_v1_2::ReportAcceptedEventV1_2::decode_log(log)?,
            ))
        } else {
            Err(anyhow!(
                "unknown topic {:?} for lane type {:?}",
                topic,
                self.lane_type
            ))
        }
    }
}

#[derive(Debug, Clone)]
pub struct DestBlessState {
    pub my_vote_address: Address,

    pub commit_store_address: Address,
    pub config_version: u32,

    pub my_voted_but_not_yet_blessed_roots: std::collections::HashSet<Bytes32>,
    pub blessed_roots: std::collections::HashSet<Bytes32>,
    pub committed_roots_with_intervals: std::vec::Vec<crate::commit_store_common::RootWithInterval>,
}

impl State for DestBlessState {
    type ContractEventUnion = DestEventUnion;
    type ContractEventFilter = VersionedDestEventFilter;
    fn update_with_event(
        &mut self,
        _block_number: u64,
        _block_hash: Bytes32,
        _address: Address,
        filter: &Self::ContractEventFilter,
        ev: DestEventUnion,
    ) -> Result<()> {
        use crate::afn_contract::{
            AlreadyBlessedEvent, AlreadyVotedToBlessEvent, TaggedRootBlessVotesResetEvent,
            TaggedRootBlessedEvent, VotedToBlessEvent,
        };
        use crate::commit_store_v1_0::ReportAcceptedEventV1_0;
        use crate::commit_store_v1_2::ReportAcceptedEventV1_2;
        match (filter.lane_type, ev) {
            (
                LaneType::Evm2EvmV1_0 | LaneType::Evm2EvmV1_2,
                DestEventUnion::Blessed(TaggedRootBlessedEvent { tagged_root, .. })
                | DestEventUnion::AlreadyBlessed(AlreadyBlessedEvent { tagged_root, .. }),
            ) => {
                if tagged_root.commit_store == self.commit_store_address {
                    self.blessed_roots.insert(tagged_root.root);
                    self.my_voted_but_not_yet_blessed_roots
                        .remove(&tagged_root.root);
                }
            }
            (
                LaneType::Evm2EvmV1_0 | LaneType::Evm2EvmV1_2,
                DestEventUnion::Voted(VotedToBlessEvent {
                    config_version,
                    voter,
                    tagged_root,
                    ..
                }),
            )
            | (
                LaneType::Evm2EvmV1_0 | LaneType::Evm2EvmV1_2,
                DestEventUnion::AlreadyVoted(AlreadyVotedToBlessEvent {
                    config_version,
                    voter,
                    tagged_root,
                    ..
                }),
            ) => {
                if config_version == self.config_version
                    && voter == self.my_vote_address
                    && tagged_root.commit_store == self.commit_store_address
                {
                    let root = tagged_root.root;
                    if !self.blessed_roots.contains(&root) {
                        self.my_voted_but_not_yet_blessed_roots.insert(root);
                    }
                }
            }
            (
                LaneType::Evm2EvmV1_0 | LaneType::Evm2EvmV1_2,
                DestEventUnion::BlessVotesReset(TaggedRootBlessVotesResetEvent {
                    config_version,
                    tagged_root,
                    ..
                }),
            ) => {
                if config_version == self.config_version
                    && tagged_root.commit_store == self.commit_store_address
                {
                    self.blessed_roots.remove(&tagged_root.root);
                    self.my_voted_but_not_yet_blessed_roots
                        .remove(&tagged_root.root);
                }
            }
            (
                LaneType::Evm2EvmV1_0,
                DestEventUnion::CommittedV1_0(ReportAcceptedEventV1_0 { report }),
            ) => {
                self.committed_roots_with_intervals.push(report.into());
            }
            (
                LaneType::Evm2EvmV1_2,
                DestEventUnion::CommittedV1_2(ReportAcceptedEventV1_2 { report }),
            ) => {
                self.committed_roots_with_intervals.push(report.into());
            }
            (ty @ LaneType::Evm2EvmV1_2, ev @ DestEventUnion::CommittedV1_0(_))
            | (ty @ LaneType::Evm2EvmV1_0, ev @ DestEventUnion::CommittedV1_2(_)) => {
                bail!("lane type mismatch: expected message of type {ty:?} but got {ev:?}",);
            }
        };
        Ok(())
    }
}

impl DestBlessState {
    pub fn unverified_roots_with_intervals_we_could_vote(
        &self,
    ) -> impl Iterator<Item = (usize, &crate::commit_store_common::RootWithInterval)> + '_ {
        self.committed_roots_with_intervals
            .iter()
            .enumerate()
            .filter(|(_, ri)| {
                !(self.blessed_roots.contains(&ri.root)
                    || self.my_voted_but_not_yet_blessed_roots.contains(&ri.root))
            })
    }
}

pub type DestBlessWorker = UnstableContractEventStateMachineWorker<DestBlessState>;

use crate::evm2evm_offramp::{ExecutionStateChanged, MessageExecutionState};

#[derive(Clone)]
pub struct SimpleExecutionStateChangedFilter;
impl ContractEventFilter for SimpleExecutionStateChangedFilter {
    type ContractEventUnion = ExecutionStateChanged;
    fn log_topics(&self) -> Vec<Bytes32> {
        vec![<Self::ContractEventUnion as LogTopic>::log_topic()]
    }

    fn decode_log(&self, log: EVMLog) -> Result<Self::ContractEventUnion> {
        <Self::ContractEventUnion as DecodeLog>::decode_log(log)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OffRampStateAnomaly {
    DoubleExecution {
        seq_nr: u64,
        old_message_id: Option<Bytes32>,
        new_message_id: Bytes32,
        new_state: MessageExecutionState,
    },
    InvalidMessageState {
        seq_nr: u64,
        message_id: Bytes32,
        state: MessageExecutionState,
    },
}

#[derive(Clone, Debug, Default)]
pub struct DestOffRampState {
    pub successfully_executed_seq_nrs: BitSet,
    pub local_anomalies: Vec<OffRampStateAnomaly>,
    pub message_id_by_sequence_number: std::collections::BTreeMap<u64, Bytes32>,
    pub max_sequence_number: Option<u64>,
}

impl DestOffRampState {
    pub fn reap_message_ids<'a>(&mut self, seq_nrs: impl Iterator<Item = &'a u64>) {
        for seq_nr in seq_nrs {
            self.message_id_by_sequence_number.remove(seq_nr);
        }
    }
}

impl State for DestOffRampState {
    type ContractEventUnion = ExecutionStateChanged;
    type ContractEventFilter = SimpleExecutionStateChangedFilter;
    fn update_with_event(
        &mut self,
        _block_number: u64,
        _block_hash: Bytes32,
        _address: Address,
        _filter: &Self::ContractEventFilter,
        ev: ExecutionStateChanged,
    ) -> Result<()> {
        let old_message_id = self
            .message_id_by_sequence_number
            .insert(ev.sequence_number, ev.message_id);
        if old_message_id.unwrap_or(ev.message_id) != ev.message_id {
            self.local_anomalies
                .push(OffRampStateAnomaly::DoubleExecution {
                    seq_nr: ev.sequence_number,
                    old_message_id,
                    new_message_id: ev.message_id,
                    new_state: ev.state,
                });
        }
        match (
            self.successfully_executed_seq_nrs
                .contains(ev.sequence_number as usize),
            ev.state,
        ) {
            (false, MessageExecutionState::Failure) => {}
            (false, MessageExecutionState::Success) => {
                self.successfully_executed_seq_nrs
                    .insert(ev.sequence_number as usize);
            }
            (false, state) => self
                .local_anomalies
                .push(OffRampStateAnomaly::InvalidMessageState {
                    seq_nr: ev.sequence_number,
                    message_id: ev.message_id,
                    state,
                }),
            (true, new_state) => {
                warn!(
                    "double execution for sequence number {} (old message id: {:?}, new message id: {:?}, new state: {:?})",
                    ev.sequence_number, old_message_id, ev.message_id, new_state
                );
                self.local_anomalies
                    .push(OffRampStateAnomaly::DoubleExecution {
                        seq_nr: ev.sequence_number,
                        old_message_id,
                        new_message_id: ev.message_id,
                        new_state: ev.state,
                    });
            }
        };
        if ev.sequence_number >= self.max_sequence_number.unwrap_or(0) {
            self.max_sequence_number = Some(ev.sequence_number);
        }
        Ok(())
    }
}

pub type DestOffRampWorker = UnstableContractEventStateMachineWorker<DestOffRampState>;
