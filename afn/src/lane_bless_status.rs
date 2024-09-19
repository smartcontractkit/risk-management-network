use crate::{
    afn_contract::TaggedRoot,
    commit_store::{verify_root_with_interval, RootWithInterval},
    common::LaneId,
    contract_event_state_machine::{
        DestBlessWorker, DestOffRampState, DestOffRampWorker, SourceBlessWorker,
    },
    worker,
};
use anyhow::Result;
use std::{
    ops::Deref,
    sync::{Arc, Mutex},
};
use tracing::{debug, trace, warn};

use super::worker::ShutdownHandle;

#[derive(Clone, Debug)]
pub struct LaneBlessStatus {
    pub verified_tagged_roots_with_not_yet_executed_msgs: Vec<TaggedRoot>,
}

pub struct LaneBlessStatusWorker {
    pub lane_id: LaneId,
    pub lane_bless_status: Arc<Mutex<Option<LaneBlessStatus>>>,
}
impl LaneBlessStatusWorker {
    pub fn spawn(
        ctx: &Arc<worker::Context>,
        lane_id: LaneId,
        source_bless_worker: Arc<SourceBlessWorker>,
        dest_bless_worker: Arc<DestBlessWorker>,
        dest_offramp_worker: Arc<DestOffRampWorker>,
        poll_interval: std::time::Duration,
    ) -> Result<(Self, ShutdownHandle)> {
        let worker_name = format!("LaneBlessStatusWorker({lane_id})");
        let lane_bless_status = Arc::new(Mutex::new(None));
        let handle = ctx.spawn_repeat(worker_name.clone(), poll_interval, {
            let lane_bless_status = Arc::clone(&lane_bless_status);
            move |_ctx| -> Result<()> {
                let latest_lane_bless_status = Self::latest_lane_bless_status(
                    &worker_name,
                    &source_bless_worker,
                    &dest_bless_worker,
                    &dest_offramp_worker,
                );
                *lane_bless_status.lock().unwrap() = Some(latest_lane_bless_status);
                Ok(())
            }
        });
        Ok((
            Self {
                lane_id,
                lane_bless_status,
            },
            handle,
        ))
    }

    fn first_not_yet_executed_message(
        dest_offramp_state_unstable: &DestOffRampState,
        root_with_interval: &RootWithInterval,
    ) -> Option<u64> {
        (root_with_interval.interval.min..=root_with_interval.interval.max).find(|&seq_nr| {
            !dest_offramp_state_unstable
                .successfully_executed_seq_nrs
                .contains(seq_nr as usize)
        })
    }
    fn latest_lane_bless_status(
        worker_name: &str,
        source_bless_worker: &SourceBlessWorker,
        dest_bless_worker: &DestBlessWorker,
        dest_offramp_worker: &DestOffRampWorker,
    ) -> LaneBlessStatus {
        let (dest_bless_state_unstable, dest_bless_state_initially_synced) =
            dest_bless_worker.initially_synced_unstable_state_read();
        let (source_bless_state_stable, source_bless_state_synced) =
            source_bless_worker.synced_stable_state_read();
        let (dest_offramp_state_unstable, dest_offramp_state_initially_synced) =
            dest_offramp_worker.initially_synced_unstable_state_read();
        let commit_store = dest_bless_state_unstable.commit_store_address;

        let mut verified_tagged_roots_with_not_yet_executed_msgs = Vec::new();

        if !dest_offramp_state_initially_synced || !dest_bless_state_initially_synced {
            trace!(
                dest_bless_state_initially_synced,
                dest_offramp_state_initially_synced,
                "{worker_name}: skipped verifying roots because dest_offramp_state or dest_bless_state is not initially synced"
            );
            return LaneBlessStatus {
                verified_tagged_roots_with_not_yet_executed_msgs,
            };
        }

        for (_root_idx, root_with_interval) in
            dest_bless_state_unstable.unverified_roots_with_intervals_we_could_vote()
        {
            let first_not_yet_executed_message = Self::first_not_yet_executed_message(
                &dest_offramp_state_unstable,
                root_with_interval,
            );
            if first_not_yet_executed_message.is_none() {
                continue;
            }
            match verify_root_with_interval(root_with_interval, source_bless_state_stable.deref()) {
                Ok(true) => {
                    debug!(
                        ?root_with_interval,
                        ?first_not_yet_executed_message,
                        "{worker_name}: verified root"
                    );
                    verified_tagged_roots_with_not_yet_executed_msgs.push(TaggedRoot {
                        commit_store,
                        root: root_with_interval.root,
                    });
                }
                Ok(false) => {
                    warn!(
                        ?root_with_interval,
                        ?first_not_yet_executed_message,
                        "{worker_name}: malicious root"
                    );
                }
                Err(err) if source_bless_state_synced => {
                    warn!(
                        ?root_with_interval,
                        ?first_not_yet_executed_message,
                        ?err,
                        "{worker_name}: not enough data to verify root even though source is synced!",
                    )
                }
                Err(_) => {}
            }
        }
        LaneBlessStatus {
            verified_tagged_roots_with_not_yet_executed_msgs,
        }
    }
}
