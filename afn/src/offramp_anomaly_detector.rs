use crate::{
    common::LaneId,
    contract_event_state_machine::DestOffRampWorker,
    contract_event_state_machine::{OffRampStateAnomaly, SourceBlessWorker},
    evm2evm_offramp::MessageId,
    worker::{self, ShutdownHandle},
};

use std::{
    collections::{BTreeSet, HashSet},
    sync::{Arc, RwLock},
    time::Instant,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OffRampAnomaly {
    Local(OffRampStateAnomaly),
    FalseExecution {
        seq_nr: u64,
        src_message_id: Option<MessageId>,
        dest_message_id: MessageId,
    },
}

#[derive(Default, Clone, Debug)]
pub struct OffRampAnomalyDetectorState {
    pub anomalies: HashSet<OffRampAnomaly>,
    pub seq_nrs_with_reapable_msg_ids: HashSet<u64>,
    pub last_checked: Option<Instant>,
}

#[derive(Default, Clone, Debug)]
pub struct OffRampAnomalyDetector {
    pub state: Arc<RwLock<OffRampAnomalyDetectorState>>,
}

impl OffRampAnomalyDetector {
    pub fn spawn(
        ctx: &Arc<worker::Context>,
        lane_id: LaneId,
        source_bless_worker: Arc<SourceBlessWorker>,
        dest_offramp_worker: Arc<DestOffRampWorker>,
        initial_state: OffRampAnomalyDetectorState,
    ) -> (Self, ShutdownHandle) {
        let worker_name = format!("OffRampAnomalyDetector({lane_id})");
        let state = Arc::new(RwLock::new(initial_state));
        let handle = ctx.spawn_repeat(worker_name.clone(),crate::config::OFFRAMP_ANOMALY_DETECTOR_POLL_INTERVAL, {
            let source_bless_worker = Arc::clone(&source_bless_worker);
            let dest_offramp_worker = Arc::clone(&dest_offramp_worker);
            let state = Arc::clone(&state);
            move |_ctx| {
                    {
                        let mut state = state.write().unwrap();
                        let (dest_state, dest_stable_state) =
                            dest_offramp_worker.unstable_and_stable_state_read();
                        if !dest_state.local_anomalies.is_empty() {
                            tracing::error!(
                                "{worker_name}: local anomalies detected! {:?}",
                                dest_state.local_anomalies
                            );
                            state.anomalies.extend(
                                dest_state
                                    .local_anomalies
                                    .iter()
                                    .map(|&a| OffRampAnomaly::Local(a)),
                            );
                        }

                        let (
                            (source_unstable_state, source_unstable_state_synced),
                            (source_stable_state, source_stable_state_synced)
                        ) =
                            source_bless_worker.synced_unstable_and_stable_state_read();

                        let mut skipped_seq_nrs: BTreeSet<u64> = BTreeSet::new();
                        for (&dest_seq_nr, &dest_message_id) in
                            &dest_state.message_id_by_sequence_number
                        {
                            if let Some(&src_message_id) = source_stable_state.message_hash(dest_seq_nr) {
                                if src_message_id != dest_message_id {
                                    tracing::error!("{worker_name}: message {dest_seq_nr} is falsified! \
                                    (src message id: {src_message_id}, dest message id: {dest_message_id})");
                                    state.anomalies.insert(OffRampAnomaly::FalseExecution {
                                        seq_nr: dest_seq_nr,
                                        src_message_id: Some(src_message_id),
                                        dest_message_id,
                                    });
                                } else if dest_stable_state
                                    .successfully_executed_seq_nrs
                                    .contains(dest_seq_nr as usize)
                                {
                                    state.seq_nrs_with_reapable_msg_ids.insert(dest_seq_nr);
                                }
                            } else if source_unstable_state_synced && source_unstable_state.message_hash(dest_seq_nr).is_none() && source_stable_state_synced {
                                tracing::error!("{worker_name}: message {dest_seq_nr} is created out of thin air! \
                                (dest message id: {dest_message_id})");
                                state.anomalies.insert(OffRampAnomaly::FalseExecution {
                                    seq_nr: dest_seq_nr,
                                    src_message_id: None,
                                    dest_message_id,
                                });
                            } else {
                                skipped_seq_nrs.insert(dest_seq_nr);
                            }
                        }

                        if let (Some(min_skipped_seq_nr), Some(max_skipped_seq_nr)) =
                            (skipped_seq_nrs.first(), skipped_seq_nrs.last())
                        {
                            tracing::warn!("{worker_name}: skipping check for messages \
                            {min_skipped_seq_nr}..{max_skipped_seq_nr} as {} state is not synced yet", lane_id.source_chain_name);
                        }
                        state.last_checked = Some(Instant::now());
                    }
                Ok(())
            }
        });
        (Self { state }, handle)
    }
}
