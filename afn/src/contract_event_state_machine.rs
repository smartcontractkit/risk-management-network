use crate::{
    cached_rpc::{CachedRpc, LogsReadError},
    chain_status::{age, ChainStatus},
    chain_status_worker::ChainStatusWorker,
    common::{ChainName, DecodeLog, LogTopic},
    config::ChainStability,
    evm2evm_onramp::CcipsendRequestedEvent,
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

pub trait ContractEventUnion: Sized {
    fn log_topics() -> Vec<Bytes32>;
    fn decode_log(log: EVMLog) -> Result<Self>;
}

pub trait State: Clone {
    type Ev: ContractEventUnion;
    fn update_with_event(
        &mut self,
        block_number: u64,
        block_hash: Bytes32,
        address: Address,
        ev: Self::Ev,
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
    max_blockrange: u64,
    max_fresh_block_age: Duration,
    addresses: Vec<Address>,
    update_progress: Arc<Mutex<UpdateProgress>>,
    timestamp: Arc<Mutex<Option<u64>>>,
    state: Arc<RwLock<S>>,
}

impl<S: State> Clone for ContractEventStateMachine<S> {
    fn clone(&self) -> Self {
        let update_progress = self.update_progress.lock().unwrap();

        Self {
            max_blockrange: self.max_blockrange,
            max_fresh_block_age: self.max_fresh_block_age,
            addresses: self.addresses.clone(),
            update_progress: Arc::new(Mutex::new(update_progress.clone())),
            timestamp: Arc::new(Mutex::new(*self.timestamp.lock().unwrap())),
            state: Arc::new(RwLock::new(self.state.write().unwrap().clone())),
        }
    }
}

impl<S: State> ContractEventStateMachine<S> {
    pub fn new(
        max_blockrange: u64,
        max_fresh_block_age: Duration,
        start_block_number: u64,
        addresses: Vec<Address>,
        initial_state: S,
    ) -> Self {
        Self {
            max_blockrange,
            max_fresh_block_age,
            addresses,
            update_progress: Arc::new(Mutex::new(UpdateProgress::NextStartBlockNumber(
                start_block_number,
            ))),
            timestamp: Arc::new(Mutex::new(None)),
            state: Arc::new(RwLock::new(initial_state)),
        }
    }

    fn fork_from(&self, other: &Self) -> Result<()> {
        if other.max_blockrange != self.max_blockrange || other.addresses != self.addresses {
            bail!("max_blockrange or addresses mismatch");
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
        rpc: &CachedRpc,
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
        let end_block_number = match finality {
            Finality::Stable => tail.stable_tip().number,
            Finality::Unstable => tip.number,
        };

        if end_block_number < start_block_number {
            *self.timestamp.lock().unwrap() = Some(tip.timestamp);
            return Ok(ContractStateMachineUpdateResult::Complete);
        }

        let (actual_end_block_number, logs) = {
            let mut candidate_end_block_number = std::cmp::min(
                end_block_number,
                start_block_number
                    .checked_add(self.max_blockrange)
                    .and_then(|x| x.checked_sub(1))
                    .ok_or(anyhow!("block number overflow"))?,
            );

            loop {
                match rpc.get_logs(
                    start_block_number,
                    candidate_end_block_number,
                    match finality {
                        Finality::Stable => true,
                        Finality::Unstable => false,
                    },
                ) {
                    Ok(logs) => break (candidate_end_block_number, logs),
                    Err(LogsReadError::NotAvailable) => bail!("logs not available"),
                    Err(LogsReadError::TooManyLogs)
                        if candidate_end_block_number == start_block_number =>
                    {
                        bail!("even a single block ({start_block_number}) contained too many logs to handle")
                    }
                    Err(LogsReadError::TooManyLogs) => {
                        tracing::debug!("block range {start_block_number}..{candidate_end_block_number} contained too many logs to handle, reducing range size");
                        candidate_end_block_number = start_block_number
                            + (candidate_end_block_number - start_block_number) / 2
                    }
                    Err(e) => bail!("got unrecoverable error while reading logs {e:?}"),
                }
            }
        };

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
                let ev = S::Ev::decode_log(log)?;
                if let Err(e) = state.update_with_event(block_number, block_hash, address, ev) {
                    *update_progress = UpdateProgress::Corrupted;
                    return Err(e);
                }
            }

            *update_progress = UpdateProgress::NextStartBlockNumber(actual_end_block_number + 1);
        }

        Ok(if end_block_number == actual_end_block_number {
            *self.timestamp.lock().unwrap() = Some(tip.timestamp);
            ContractStateMachineUpdateResult::Complete
        } else {
            ContractStateMachineUpdateResult::Partial {
                block_number: actual_end_block_number,
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

const MAX_UNSTABLE_UPDATE_CONSECUTIVE_FAILS: u64 = 10;

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
        let stable_cesm = Arc::new(cesm);
        let unstable_cesm = Arc::new(stable_cesm.as_ref().clone());
        let handle = ctx.spawn(worker_name.to_string(), {
            let stable_cesm = Arc::clone(&stable_cesm);
            let unstable_cesm = Arc::clone(&unstable_cesm);
            move |ctx, worker_name| -> Result<()> {
                let cached_rpc = CachedRpc::new(
                    chain,
                    &stable_cesm.addresses,
                    S::Ev::log_topics(),
                    rpc,
                    chain_stability,
                    worker_name,
                )?;
                let mut unstable_update_consecutive_fails = 0;
                let mut reported_initial_sync = false;
                let worker_name = worker_name.to_owned();
                ctx.repeat(poll_interval, move |ctx| {
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
                                unstable_update_consecutive_fails = 0;
                                unstable_cesm.fork_from(&new_unstable_cesm).unwrap();
                            }
                            Err(e) => {
                                unstable_update_consecutive_fails += 1;
                                warn!("{worker_name} unstable update failed ({unstable_update_consecutive_fails} consecutive time(s)): {e}");
                                if unstable_update_consecutive_fails
                                    > MAX_UNSTABLE_UPDATE_CONSECUTIVE_FAILS
                                {
                                    return Err(e)
                                .with_context(|| format!("{worker_name} unstable update failed {unstable_update_consecutive_fails} consecutive times"));
                                }
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

impl ContractEventUnion for CcipsendRequestedEvent {
    fn log_topics() -> Vec<Bytes32> {
        vec![<Self as LogTopic>::log_topic()]
    }

    fn decode_log(log: EVMLog) -> Result<Self> {
        <Self as DecodeLog>::decode_log(log)
    }
}

#[derive(Clone)]
pub struct SourceBlessState {
    pub message_metadata: crate::hashable::MessageMetadata,
    pub seq_nrs_and_message_hashes: std::collections::VecDeque<(u64, Bytes32)>,
}

impl State for SourceBlessState {
    type Ev = CcipsendRequestedEvent;
    fn update_with_event(
        &mut self,
        _block_number: u64,
        _block_hash: Bytes32,
        _address: Address,
        ev: CcipsendRequestedEvent,
    ) -> Result<()> {
        use crate::hashable::Hashable;

        let msg = ev.message;
        let seq_nr = msg.sequence_number;
        let msg_hash = msg.hash(&self.message_metadata);
        if msg_hash != msg.message_id {
            bail!(
                "hash mismatch for seq_nr {seq_nr}! we computed: {msg_hash}, event contains: {}",
                msg.message_id
            );
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
    fn get_message_hashes(&self, interval: &crate::commit_store::Interval) -> Option<Vec<Bytes32>> {
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

pub enum DestEventUnion {
    Blessed(crate::afn_contract::TaggedRootBlessedEvent),
    BlessVotesReset(crate::afn_contract::TaggedRootBlessVotesResetEvent),
    AlreadyBlessed(crate::afn_contract::AlreadyBlessedEvent),
    Voted(crate::afn_contract::VotedToBlessEvent),
    AlreadyVoted(crate::afn_contract::AlreadyVotedToBlessEvent),
    Committed(crate::commit_store::ReportAcceptedEvent),
}

impl ContractEventUnion for DestEventUnion {
    fn log_topics() -> Vec<Bytes32> {
        vec![
            crate::afn_contract::TaggedRootBlessedEvent::log_topic(),
            crate::afn_contract::TaggedRootBlessVotesResetEvent::log_topic(),
            crate::afn_contract::AlreadyBlessedEvent::log_topic(),
            crate::afn_contract::VotedToBlessEvent::log_topic(),
            crate::afn_contract::AlreadyVotedToBlessEvent::log_topic(),
            crate::commit_store::ReportAcceptedEvent::log_topic(),
        ]
    }

    fn decode_log(log: EVMLog) -> Result<Self> {
        let &topic = log
            .topics
            .get(0)
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
        } else if topic == crate::commit_store::ReportAcceptedEvent::log_topic() {
            Ok(DestEventUnion::Committed(
                crate::commit_store::ReportAcceptedEvent::decode_log(log)?,
            ))
        } else {
            Err(anyhow!("unknown topic {:?}", topic))
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
    pub committed_roots_with_intervals: std::vec::Vec<crate::commit_report::RootWithInterval>,
}

impl State for DestBlessState {
    type Ev = DestEventUnion;
    fn update_with_event(
        &mut self,
        _block_number: u64,
        _block_hash: Bytes32,
        _address: Address,
        ev: DestEventUnion,
    ) -> Result<()> {
        use crate::afn_contract::{
            AlreadyBlessedEvent, AlreadyVotedToBlessEvent, TaggedRootBlessVotesResetEvent,
            TaggedRootBlessedEvent, VotedToBlessEvent,
        };
        use crate::commit_store::ReportAcceptedEvent;
        match ev {
            DestEventUnion::Blessed(TaggedRootBlessedEvent { tagged_root, .. })
            | DestEventUnion::AlreadyBlessed(AlreadyBlessedEvent { tagged_root, .. }) => {
                if tagged_root.commit_store == self.commit_store_address {
                    self.blessed_roots.insert(tagged_root.root);
                    self.my_voted_but_not_yet_blessed_roots
                        .remove(&tagged_root.root);
                }
            }
            DestEventUnion::Voted(VotedToBlessEvent {
                config_version,
                voter,
                tagged_root,
                ..
            })
            | DestEventUnion::AlreadyVoted(AlreadyVotedToBlessEvent {
                config_version,
                voter,
                tagged_root,
                ..
            }) => {
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
            DestEventUnion::BlessVotesReset(TaggedRootBlessVotesResetEvent {
                config_version,
                tagged_root,
                ..
            }) => {
                if config_version == self.config_version
                    && tagged_root.commit_store == self.commit_store_address
                {
                    self.blessed_roots.remove(&tagged_root.root);
                    self.my_voted_but_not_yet_blessed_roots
                        .remove(&tagged_root.root);
                }
            }
            DestEventUnion::Committed(ReportAcceptedEvent { report }) => {
                self.committed_roots_with_intervals.push(report.into());
            }
        };
        Ok(())
    }
}

impl DestBlessState {
    pub fn unverified_roots_with_intervals_we_could_vote(
        &self,
    ) -> impl Iterator<Item = (usize, &crate::commit_report::RootWithInterval)> + '_ {
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

impl ContractEventUnion for ExecutionStateChanged {
    fn log_topics() -> Vec<Bytes32> {
        vec![<Self as LogTopic>::log_topic()]
    }

    fn decode_log(log: EVMLog) -> Result<Self> {
        <Self as DecodeLog>::decode_log(log)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OffRampStateAnomaly {
    DoubleExecution {
        seq_nr: u64,
        old_message_id: Bytes32,
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
    type Ev = ExecutionStateChanged;
    fn update_with_event(
        &mut self,
        _block_number: u64,
        _block_hash: Bytes32,
        _address: Address,
        ev: ExecutionStateChanged,
    ) -> Result<()> {
        let old_message_id = match self.message_id_by_sequence_number.get(&ev.sequence_number) {
            Some(old_message_id) => {
                if old_message_id != &ev.message_id {
                    self.local_anomalies
                        .push(OffRampStateAnomaly::DoubleExecution {
                            seq_nr: ev.sequence_number,
                            old_message_id: *old_message_id,
                            new_message_id: ev.message_id,
                            new_state: ev.state,
                        });
                }
                Some(old_message_id)
            }
            None => {
                self.message_id_by_sequence_number
                    .insert(ev.sequence_number, ev.message_id);
                None
            }
        };
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
                let old_message_id = *old_message_id.unwrap();
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
