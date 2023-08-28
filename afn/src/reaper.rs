use std::{collections::HashMap, sync::Arc};

use crate::{
    common::LaneId,
    config,
    contract_event_state_machine::{DestOffRampWorker, SourceBlessWorker},
    offramp_anomaly_detector::OffRampAnomalyDetector,
    worker::{Context, ShutdownHandle},
};

pub struct ReapableLaneWorkers {
    pub offramp_anomaly_detector: Arc<OffRampAnomalyDetector>,
    pub source_bless_worker: Arc<SourceBlessWorker>,
    pub dest_offramp_worker: Arc<DestOffRampWorker>,
}

fn reap(lane_id: &LaneId, workers: &ReapableLaneWorkers) {
    let reapable_dest_offramp_seq_nrs = {
        let mut anomaly_detector_state = workers.offramp_anomaly_detector.state.write().unwrap();
        std::mem::take(&mut anomaly_detector_state.seq_nrs_with_reapable_msg_ids)
    };
    {
        let mut offramp_state = workers.dest_offramp_worker.stable_state_mut();
        offramp_state.reap_message_ids(reapable_dest_offramp_seq_nrs.iter());
    }
    tracing::info!(
        num_reaped_msg_ids = reapable_dest_offramp_seq_nrs.len(),
        "reaped for {lane_id}",
    );
}

pub fn spawn_reaper(
    ctx: Arc<Context>,
    reapable_lane_workers: HashMap<LaneId, ReapableLaneWorkers>,
) -> ((), ShutdownHandle) {
    let handle = ctx.spawn_repeat("Reaper", config::REAPER_POLL_INTERVAL, move |_ctx| {
        for (lane_id, workers) in reapable_lane_workers.iter() {
            reap(lane_id, workers);
        }
        Ok(())
    });
    ((), handle)
}
