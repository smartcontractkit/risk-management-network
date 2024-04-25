use crate::{
    afn_contract::AFNInterface,
    common::ChainName,
    config::SharedChainConfig,
    curse_beacon::{CurseBeacon, CurseId},
    key_types::SecretKey,
    worker,
};
use anyhow::Result;
use minieth::{rpc::Rpc, tx_sender::TransactionSender};
use std::{
    collections::HashMap,
    fs::File,
    io::Write,
    sync::{Arc, Mutex},
    time::Instant,
};
use tracing::{error, info, warn};

use super::{afn_voting_manager::VotingMode, worker::ShutdownHandle};

pub struct VoteToCurseWorker {
    pub chain_name: ChainName,
}

impl VoteToCurseWorker {
    pub fn spawn(
        ctx: &Arc<worker::Context>,
        rpc: Arc<Rpc>,
        config: &SharedChainConfig,
        poll_interval: std::time::Duration,
        key: SecretKey,
        curse_beacon: Arc<CurseBeacon>,
        mode: VotingMode,
        curse_file: Option<Arc<Mutex<File>>>,
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
                let mut do_not_send_vote_for_curse_id_until: HashMap<CurseId, Instant> = HashMap::new();

                let worker_name = worker_name.to_owned();
                ctx.repeat(worker_name.clone(), poll_interval, move |_ctx| {
                    for (anomaly, curse_id) in curse_beacon.get_anomalies_and_curse_ids_for_chain(worker.chain_name) {
                        if let Some(do_not_send_until) = do_not_send_vote_for_curse_id_until.get(&curse_id) {
                            if Instant::now() < *do_not_send_until {
                                warn!(
                                    "{worker_name}: skipped voting to curse for anomaly {:?} with curse id {}: \
                                    voted to curse too recently, can't revote for {:?}",
                                    anomaly, curse_id, do_not_send_until.saturating_duration_since(Instant::now())
                                );
                                continue;
                            }
                        }

                        error!(
                            "{worker_name}: voting to curse due to anomaly {:?} with curse id {}",
                            anomaly, curse_id
                        );
                        match mode {
                            VotingMode::DryRun | VotingMode::Passive => {
                                error!(
                                    "{worker_name}: would be voting to curse due to anomaly {:?} with curse id {} (if not in mode {:?})",
                                    anomaly, curse_id, mode
                                );
                            }
                            VotingMode::UnreliableRemote => {
                                error!(
                                    "{worker_name}: would be voting to curse due to anomaly {:?} with curse id {} (if not in mode {:?})",
                                    anomaly, curse_id, mode
                                );

                                if let Some(ref curse_file) = curse_file {
                                    let mut curse_file = curse_file.lock().unwrap();
                                    curse_file.write_all(
                                        format!("{:?} {worker_name}: voted to curse due to anomaly {:?}\n", 
                                        chrono::Utc::now(), anomaly).as_bytes(),
                                    )?;
                                    curse_file.flush()?;
                                }
                            }
                            VotingMode::Active => {
                                error!(
                                    "{worker_name}: voting to curse due to anomaly {:?} with curse id {}",
                                    anomaly, curse_id
                                );
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
                                        do_not_send_vote_for_curse_id_until.insert(
                                            curse_id,
                                            Instant::now()+crate::config::VOTE_TO_CURSE_SEND_INTERVAL_PER_CURSE_ID
                                        );
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
