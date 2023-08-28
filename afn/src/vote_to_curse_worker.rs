use crate::{
    afn_contract::AFNInterface, common::ChainName, config::ChainConfig, curse_beacon::CurseBeacon,
    key_types::SecretKey, worker,
};
use anyhow::Result;
use minieth::{rpc::Rpc, tx_sender::TransactionSender};
use std::sync::Arc;
use tracing::{error, info, warn};

use super::{afn_voting_manager::VotingMode, worker::ShutdownHandle};

pub struct VoteToCurseWorker {
    pub chain_name: ChainName,
}

impl VoteToCurseWorker {
    pub fn spawn(
        ctx: &Arc<worker::Context>,
        rpc: Arc<Rpc>,
        config: &ChainConfig,
        poll_interval: std::time::Duration,
        key: SecretKey,
        curse_beacon: Arc<CurseBeacon>,
        mode: VotingMode,
    ) -> Result<(Self, ShutdownHandle)> {
        let worker = Self {
            chain_name: config.name,
        };
        let config = config.clone();
        let worker_name = format!("VoteToCurseWorker({},{})", config.name, config.afn_contract);
        let handle = ctx.spawn(worker_name, {
            move |ctx, worker_name| -> Result<()> {
                let afn_contract =
                    AFNInterface::create_from_chain_config(Arc::clone(&rpc), &config)?;
                let mut tx_sender = TransactionSender::new(
                    config.name.chain_id(),
                    config.curse_fee_config,
                    Arc::clone(&rpc),
                    Arc::new(key.local_signer()),
                );

                let worker_name = worker_name.to_owned();
                ctx.repeat(poll_interval, move |_ctx| {
                    for (anomaly, curse_id) in curse_beacon.get_anomalies_and_curse_ids() {
                        error!(
                            "{worker_name}: voting to curse due to anomaly {:?} with curse id {}",
                            anomaly, curse_id
                        );
                        match mode {
                            VotingMode::DryRun | VotingMode::Passive => {}
                            VotingMode::Active => {
                                match afn_contract.send_vote_to_curse(&mut tx_sender, curse_id) {
                                    Err(err) => {
                                        warn!(
                                            "{worker_name}: failed to vote to curse due to anomaly {:?} \
                                            with curse id {} with error {:?}; \
                                            check the error: if execution reverted, we likely already managed to \
                                            send a vote for this curse id and this is to be expected",
                                            anomaly, curse_id, err
                                        );
                                    }
                                    Ok(txid) => {
                                        info!("{worker_name}: voted to curse due to anomaly {:?} with txid {}",
                                        anomaly, txid);
                                    }
                                }
                            }
                        }
                    }
                    Ok(())
                })
            }
        });
        Ok((worker, handle))
    }
}
