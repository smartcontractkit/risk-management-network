use crate::{
    chain_selector::chain_selector,
    chain_status_worker::ChainStatusWorker,
    common::LaneId,
    config::{ChainConfig, LaneConfig},
    contract_event_state_machine::{DestOffRampState, DestOffRampWorker},
    hashable,
    offramp_anomaly_detector::{OffRampAnomalyDetector, OffRampAnomalyDetectorState},
    worker::{self, ShutdownHandleGroup},
    {
        contract_event_state_machine::{
            ContractEventStateMachine, DestBlessState, DestBlessWorker, SourceBlessState,
            SourceBlessWorker,
        },
        lane_bless_status::LaneBlessStatusWorker,
    },
};
use anyhow::Result;
use minieth::{bytes::Address, rpc::Rpc};
use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
};

pub struct LaneState {
    pub lane_id: LaneId,
    pub source_bless_worker: Arc<SourceBlessWorker>,
    pub dest_bless_worker: Arc<DestBlessWorker>,
    pub lane_bless_status_worker: Arc<LaneBlessStatusWorker>,
    pub dest_offramp_worker: Arc<DestOffRampWorker>,
    pub offramp_anomaly_detector: Arc<OffRampAnomalyDetector>,
}

impl LaneState {
    pub fn new_and_spawn_workers(
        ctx: &Arc<worker::Context>,
        source_rpc: Arc<Rpc>,
        dest_rpc: Arc<Rpc>,
        lane_config: &LaneConfig,
        source_chain_config: &ChainConfig,
        dest_chain_config: &ChainConfig,
        dest_chain_onchain_config_version: u32,
        dest_chain_vote_to_bless_address: Address,
        source_chain_status_worker: Arc<ChainStatusWorker>,
        dest_chain_status_worker: Arc<ChainStatusWorker>,
    ) -> Result<(Self, ShutdownHandleGroup)> {
        let lane_id = lane_config.lane_id.clone();
        let mut shutdown_handles = ShutdownHandleGroup::default();
        let source_bless_worker = Arc::new(shutdown_handles.add(SourceBlessWorker::spawn(
            ctx,
            &format!("SourceBlessWorker({})", lane_id),
            crate::config::CESM_POLL_INTERVAL,
            ContractEventStateMachine::new(
                source_chain_config.source_contracts_getlogs_max_block_range,
                source_chain_config.max_fresh_block_age.into(),
                lane_config.source_start_block_number,
                vec![lane_config.on_ramp],
                SourceBlessState {
                    message_metadata: hashable::MessageMetadata {
                        onramp_address: lane_config.on_ramp,
                        source_chain_selector: chain_selector(source_chain_config.name),
                        dest_chain_selector: chain_selector(dest_chain_config.name),
                    },
                    seq_nrs_and_message_hashes: VecDeque::new(),
                },
            ),
            source_chain_config.name,
            source_rpc,
            source_chain_status_worker,
            source_chain_config.stability,
        )));
        let dest_bless_worker = Arc::new(shutdown_handles.add(DestBlessWorker::spawn(
            ctx,
            &format!("DestBlessWorker({})", lane_id),
            crate::config::CESM_POLL_INTERVAL,
            ContractEventStateMachine::new(
                dest_chain_config.dest_contracts_getlogs_max_block_range,
                dest_chain_config.max_fresh_block_age.into(),
                lane_config.dest_start_block_number,
                vec![lane_config.commit_store, dest_chain_config.afn_contract],
                DestBlessState {
                    my_vote_address: dest_chain_vote_to_bless_address,
                    commit_store_address: lane_config.commit_store,
                    config_version: dest_chain_onchain_config_version,
                    my_voted_but_not_yet_blessed_roots: HashSet::new(),
                    blessed_roots: HashSet::new(),
                    committed_roots_with_intervals: Vec::new(),
                },
            ),
            dest_chain_config.name,
            Arc::clone(&dest_rpc),
            Arc::clone(&dest_chain_status_worker),
            dest_chain_config.stability,
        )));
        let lane_bless_status_worker =
            Arc::new(shutdown_handles.add(LaneBlessStatusWorker::spawn(
                ctx,
                lane_id.clone(),
                Arc::clone(&source_bless_worker),
                Arc::clone(&dest_bless_worker),
                crate::config::LANE_BLESS_STATUS_POLL_INTERVAL,
            )?));
        let dest_offramp_worker = Arc::new(shutdown_handles.add(DestOffRampWorker::spawn(
            ctx,
            &format!("DestOffRampWorker({})", lane_id),
            crate::config::CESM_POLL_INTERVAL,
            ContractEventStateMachine::new(
                dest_chain_config.source_contracts_getlogs_max_block_range,
                dest_chain_config.max_fresh_block_age.into(),
                lane_config.dest_start_block_number,
                vec![lane_config.off_ramp],
                DestOffRampState::default(),
            ),
            dest_chain_config.name,
            Arc::clone(&dest_rpc),
            Arc::clone(&dest_chain_status_worker),
            dest_chain_config.stability,
        )));

        let offramp_anomaly_detector =
            Arc::new(shutdown_handles.add(OffRampAnomalyDetector::spawn(
                ctx,
                lane_id.clone(),
                Arc::clone(&source_bless_worker),
                Arc::clone(&dest_offramp_worker),
                OffRampAnomalyDetectorState::default(),
            )));

        Ok((
            Self {
                lane_id,
                source_bless_worker,
                dest_bless_worker,
                lane_bless_status_worker,
                dest_offramp_worker,
                offramp_anomaly_detector,
            },
            shutdown_handles,
        ))
    }
}
