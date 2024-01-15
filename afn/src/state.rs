use super::{
    afn_contract::VersionedOnchainConfig,
    afn_voting_manager::{AFNVotingManager, VotingMode},
    lane_state::LaneState,
};
use crate::{
    chain_selector::chain_selector,
    chain_state::ChainState,
    common::{ChainName, LaneId},
    config::{LaneConfig, OffchainConfig},
    config_sanity_check::{
        onchain_commit_store_static_config, onchain_offramp_static_config,
        onchain_onramp_static_config, onchain_type_and_version, CommitStoreStaticConfig,
        OffRampStaticConfig, OnRampStaticConfig,
    },
    curse_beacon::{CursableAnomaliesWithCurseIds, CursableAnomaly, CurseBeacon, CurseId},
    key_types::BlessCurseKeysByChain,
    metrics::MetricsWorker,
    reaper::{spawn_reaper, ReapableLaneWorkers},
    worker::{self, ShutdownHandleGroup},
};
use anyhow::{anyhow, bail, Context, Result};
use minieth::rpc::Rpc;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, RwLock},
    thread,
    time::Instant,
};
use tracing::{debug, info};

pub struct State {
    pub config: OffchainConfig,
    pub rpcs: HashMap<ChainName, Arc<Rpc>>,
    pub chain_states: HashMap<ChainName, ChainState>,
    pub lane_states: HashMap<LaneId, LaneState>,
    pub curse_beacon: Arc<CurseBeacon>,
    pub afn_voting_managers: HashMap<ChainName, AFNVotingManager>,
    pub onchain_configs: HashMap<ChainName, VersionedOnchainConfig>,
    pub shutdown_handles: ShutdownHandleGroup,
    pub ctx: Arc<worker::Context>,
}

pub enum MonitorResult {
    WorkersDied(Vec<(String, anyhow::Result<()>)>),
    OnchainConfigChanged,
}

fn get_onchain_configs(
    config: &OffchainConfig,
    chain_states: &HashMap<ChainName, ChainState>,
) -> HashMap<ChainName, VersionedOnchainConfig> {
    let mut onchain_configs = HashMap::new();
    for (chain_name, _) in config.dest_chains() {
        let config_discoverer = &chain_states
            .get(&chain_name)
            .unwrap()
            .config_discovery_worker;
        let versioned_onchain_config = loop {
            match config_discoverer.latest() {
                None => {
                    info!(
                        "AFN({}, {}) onchain config not discovered yet",
                        chain_name, config_discoverer.afn_address
                    );
                }
                Some(config_details) => {
                    debug!(
                        "AFN({}, {}) has onchain config {:?}",
                        chain_name, config_discoverer.afn_address, config_details
                    );
                    break config_details;
                }
            }
            thread::sleep(crate::config::CONFIG_DISCOVERY_RETRY_INTERVAL);
        };
        onchain_configs.insert(chain_name, versioned_onchain_config);
    }
    onchain_configs
}

fn validate_voter_addresses(
    config: &OffchainConfig,
    keys: &BlessCurseKeysByChain,
    onchain_configs: &HashMap<ChainName, VersionedOnchainConfig>,
) -> Result<()> {
    tracing::info!("validating voter addresses...");
    for (chain_name, _) in config.dest_chains() {
        tracing::info!("validating voter addresses on chain {}...", chain_name);
        let versioned_onchain_config = onchain_configs
            .get(&chain_name)
            .ok_or_else(|| anyhow!("onchain config not found: {chain_name}"))?;
        let keys = keys
            .get(chain_name)
            .ok_or_else(|| anyhow!("key not found: {chain_name}"))?;
        let vote_to_bless_address = keys.bless.address();
        let vote_to_curse_address = keys.curse.address();
        let matching_voter_found = versioned_onchain_config.config.voters.iter().any(|voter| {
            voter.bless_vote_addr == vote_to_bless_address
                && voter.curse_vote_addr == vote_to_curse_address
        });

        if !matching_voter_found {
            bail!(
                "no matching voter found for ({}, {}) on chain {}",
                vote_to_bless_address,
                vote_to_curse_address,
                chain_name
            )
        }
    }
    Ok(())
}

fn validate_onchain_lane_config(
    lane_config: &LaneConfig,
    rpcs: &HashMap<ChainName, Arc<Rpc>>,
) -> anyhow::Result<()> {
    let src_chain = lane_config.lane_id.source_chain_name;
    let dest_chain = lane_config.lane_id.dest_chain_name;
    let src_chain_selector = chain_selector(src_chain);
    let dest_chain_selector = chain_selector(dest_chain);
    let LaneConfig {
        on_ramp,
        commit_store,
        off_ramp,
        lane_type,
        ..
    } = *lane_config;
    use crate::config::LaneType::*;
    std::thread::scope(|s| {
        let handles = vec![
        s.spawn(move || {
            let onchain_onramp_config = onchain_onramp_static_config(&rpcs[&src_chain], on_ramp)?;
            let expected_onchain_onramp_config = OnRampStaticConfig {
                chain_selector: src_chain_selector,
                dest_chain_selector,
            };
            if onchain_onramp_config != expected_onchain_onramp_config {
                bail!("unexpected onchain onramp config: expected {expected_onchain_onramp_config:?}, found {onchain_onramp_config:?}")
            }
            Ok(())
        }),
        s.spawn(move || {
            let onchain_onramp_type_and_version = onchain_type_and_version(&rpcs[&src_chain], on_ramp)?;
            match (&onchain_onramp_type_and_version as &str, lane_type) {
                ("EVM2EVMOnRamp 1.0.0" | "EVM2EVMOnRamp 1.1.0", Evm2EvmV1_0) | ("EVM2EVMOnRamp 1.2.0", Evm2EvmV1_2) => Ok(()),
                _ => {
                    bail!("unexpected onchain onramp type and version {onchain_onramp_type_and_version:?} for lane type {lane_type:?}");
                }
            }
        }),
        s.spawn(move ||{
            let onchain_offramp_config =
                onchain_offramp_static_config(&rpcs[&dest_chain], off_ramp)?;
            let expected_onchain_offramp_config = OffRampStaticConfig {
                chain_selector: dest_chain_selector,
                src_chain_selector,
                commit_store,
                on_ramp,
            };
            if onchain_offramp_config != expected_onchain_offramp_config {
                bail!("unexpected onchain offramp config: expected {expected_onchain_offramp_config:?}, found {onchain_offramp_config:?}")
            }
            Ok(())
        }),
        s.spawn(move || {
            let onchain_offramp_type_and_version = onchain_type_and_version(&rpcs[&dest_chain], off_ramp)?;
            match (&onchain_offramp_type_and_version as &str, lane_type) {
                ("EVM2EVMOffRamp 1.0.0" | "EVM2EVMOffRamp 1.1.0", Evm2EvmV1_0) | ("EVM2EVMOffRamp 1.2.0", Evm2EvmV1_2) => Ok(()),
                _ => {
                    bail!("unexpected onchain offramp type and version {onchain_offramp_type_and_version:?} for lane type {lane_type:?}");
                }
            }
        }),
        s.spawn(move ||{
            let onchain_commit_store_config =
                onchain_commit_store_static_config(&rpcs[&dest_chain], commit_store)?;
            let expected_onchain_commit_store_config = CommitStoreStaticConfig {
                chain_selector: dest_chain_selector,
                src_chain_selector,
                on_ramp,
            };
            if onchain_commit_store_config != expected_onchain_commit_store_config {
                bail!("unexpected onchain commit store config: expected {expected_onchain_commit_store_config:?}, found {onchain_commit_store_config:?}")
            }
            Ok(())
        }),
        s.spawn(move || {
            let onchain_commit_store_type_and_version = onchain_type_and_version(&rpcs[&dest_chain], commit_store)?;
            match (&onchain_commit_store_type_and_version as &str, lane_type) {
                ("CommitStore 1.0.0" | "CommitStore 1.1.0", Evm2EvmV1_0) | ("CommitStore 1.2.0", Evm2EvmV1_2) => Ok(()),
                _ => {
                    bail!("unexpected onchain commit store type and version {onchain_commit_store_type_and_version:?} for lane type {lane_type:?}");
                }
            }
        }),
        ];
        handles
            .into_iter()
            .try_for_each(|handle| handle.join().unwrap())
    })
}

fn validate_onchain_configs(
    config: &OffchainConfig,
    rpcs: &HashMap<ChainName, Arc<Rpc>>,
) -> anyhow::Result<()> {
    tracing::info!("validating on-chain configs...");
    std::thread::scope(|s| {
        let handles = config
            .lanes
            .values()
            .map(|lane_config| {
                s.spawn(move || {
                    tracing::info!("validating lane config {lane_config:?}...");
                    validate_onchain_lane_config(lane_config, rpcs)
                        .with_context(|| format!("failed to validate lane config {lane_config:?}"))
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .try_for_each(|handle| handle.join().unwrap())
    })
}

impl State {
    pub fn new_and_spawn_workers(
        ctx: Arc<worker::Context>,
        config: OffchainConfig,
        keys: BlessCurseKeysByChain,
        mode: VotingMode,
        manual_curse: Option<CurseId>,
        metrics_file_path: Option<PathBuf>,
    ) -> Result<Self> {
        if let Some(manual_curse_id) = manual_curse {
            for countdown in (1..=5).rev() {
                tracing::error!("You have started the ARM node in MANUAL CURSE MODE with curse id {manual_curse_id}. This is a potentially DESTRUCTIVE ACTION. Starting in {countdown}...");
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }

        let mut shutdown_handles = ShutdownHandleGroup::default();
        let mut rpcs = HashMap::new();
        for (&chain_name, chain_config) in config.chains.iter() {
            rpcs.insert(
                chain_name,
                Arc::new(Rpc::new_with_multiple_urls(
                    chain_name as u64,
                    &chain_config.rpcs,
                )?),
            );
        }

        validate_onchain_configs(&config, &rpcs).context("failed to validate onchain configs")?;

        let metrics_worker = Arc::new(shutdown_handles.add(MetricsWorker::spawn(
            Arc::clone(&ctx),
            config.chains.keys().cloned().collect(),
            metrics_file_path,
        )?));

        let mut chain_states = HashMap::new();
        for (&chain_name, chain_config) in config.chains.iter() {
            let chain_state = shutdown_handles.add_group(ChainState::new_and_spawn_workers(
                &ctx,
                Arc::clone(rpcs.get(&chain_name).unwrap()),
                chain_config,
                Box::new(metrics_worker.make_chain_metrics_handle(chain_name)),
            )?);
            chain_states.insert(chain_name, chain_state);
        }
        let onchain_configs = get_onchain_configs(&config, &chain_states);
        info!(?onchain_configs, "starting with onchain configs");

        match mode {
            VotingMode::Active | VotingMode::DryRun => {
                validate_voter_addresses(&config, &keys, &onchain_configs)?;
            }
            VotingMode::Passive => {}
        }

        let mut lane_states = HashMap::new();
        for (lane_id, lane_config) in config.lanes.iter() {
            let source_chain_config =
                config
                    .chains
                    .get(&lane_id.source_chain_name)
                    .ok_or_else(|| {
                        anyhow!(
                            "source chain config not found: {}",
                            lane_id.source_chain_name
                        )
                    })?;
            let dest_chain_config =
                config.chains.get(&lane_id.dest_chain_name).ok_or_else(|| {
                    anyhow!("dest chain config not found: {}", lane_id.dest_chain_name)
                })?;
            let source_chain_status_worker = Arc::clone(
                &chain_states
                    .get(&lane_id.source_chain_name)
                    .ok_or_else(|| {
                        anyhow!(
                            "source chain state not found: {}",
                            lane_id.source_chain_name
                        )
                    })?
                    .chain_status_worker,
            );
            let dest_chain_status_worker = Arc::clone(
                &chain_states
                    .get(&lane_id.dest_chain_name)
                    .ok_or_else(|| {
                        anyhow!("dest chain state not found: {}", lane_id.dest_chain_name)
                    })?
                    .chain_status_worker,
            );
            let dest_chain_onchain_config = onchain_configs
                .get(&lane_id.dest_chain_name)
                .ok_or_else(|| {
                    anyhow!(
                        "dest chain onchain config not found: {}",
                        lane_id.dest_chain_name
                    )
                })?;
            let lane_state = shutdown_handles.add_group(LaneState::new_and_spawn_workers(
                &ctx,
                Arc::clone(rpcs.get(&lane_id.source_chain_name).unwrap()),
                Arc::clone(rpcs.get(&lane_id.dest_chain_name).unwrap()),
                lane_config,
                source_chain_config,
                dest_chain_config,
                dest_chain_onchain_config.version,
                keys.get(lane_id.dest_chain_name).unwrap().bless.address(),
                source_chain_status_worker,
                dest_chain_status_worker,
            )?);
            lane_states.insert(lane_id.clone(), lane_state);
        }

        let curse_beacon = Arc::new({
            let chain_status_workers = chain_states
                .iter()
                .map(|(chain, chain_state)| (*chain, Arc::clone(&chain_state.chain_status_worker)))
                .collect();
            let offramp_anomaly_detectors = lane_states
                .iter()
                .map(|(lane_id, ls)| (lane_id.clone(), Arc::clone(&ls.offramp_anomaly_detector)))
                .collect();
            let mut cursable_anomalies = CursableAnomaliesWithCurseIds::default();
            if let Some(curse_id) = manual_curse {
                cursable_anomalies.note(CursableAnomaly::ManualCurse(curse_id));
            }
            CurseBeacon {
                chain_status_workers,
                offramp_anomaly_detectors,
                cursable_anomalies: Arc::new(RwLock::new(cursable_anomalies)),
            }
        });

        let mut afn_voting_managers = HashMap::new();
        for (chain_name, chain_config) in config.dest_chains() {
            let lane_bless_status_workers = lane_states
                .iter()
                .filter(|&(lane_id, _)| lane_id.dest_chain_name == chain_name)
                .map(|(lane_id, lane_state)| {
                    (
                        lane_id.clone(),
                        Arc::clone(&lane_state.lane_bless_status_worker),
                    )
                })
                .collect();
            let afn_voting_manager =
                shutdown_handles.add_group(AFNVotingManager::new_and_spawn_workers(
                    &ctx,
                    Arc::clone(rpcs.get(&chain_name).unwrap()),
                    chain_config,
                    onchain_configs.get(&chain_name).unwrap().config.clone(),
                    keys.get(chain_name).unwrap(),
                    lane_bless_status_workers,
                    Arc::clone(&curse_beacon),
                    mode,
                    Box::new(metrics_worker.make_chain_metrics_handle(chain_name)),
                )?);
            afn_voting_managers.insert(chain_name, afn_voting_manager);
        }
        shutdown_handles.add(spawn_reaper(
            Arc::clone(&ctx),
            lane_states
                .iter()
                .map(|(lane_id, lane_state)| {
                    (
                        lane_id.clone(),
                        ReapableLaneWorkers {
                            offramp_anomaly_detector: Arc::clone(
                                &lane_state.offramp_anomaly_detector,
                            ),
                            source_bless_worker: Arc::clone(&lane_state.source_bless_worker),
                            dest_offramp_worker: Arc::clone(&lane_state.dest_offramp_worker),
                        },
                    )
                })
                .collect(),
        ));
        Ok(Self {
            config,
            rpcs,
            chain_states,
            lane_states,
            afn_voting_managers,
            onchain_configs,
            curse_beacon,
            shutdown_handles,
            ctx,
        })
    }

    fn print_status_report(&self) {
        for (chain_name, chain_state) in &self.chain_states {
            let status = chain_state.chain_status_worker.latest_chain_status();
            if let Some(status) = status {
                let age = status.age();
                info!(%status, ?age, "{chain_name} status");
            } else {
                info!("{chain_name} status uninitialized");
            }
        }
        for (lane_id, lane_state) in &self.lane_states {
            let (first_msg, last_msg) = {
                let source_bless_state = lane_state.source_bless_worker.stable_state_read();
                let first_msg = source_bless_state
                    .seq_nrs_and_message_hashes
                    .front()
                    .cloned();
                let last_msg = source_bless_state
                    .seq_nrs_and_message_hashes
                    .back()
                    .cloned();
                (first_msg, last_msg)
            };
            let (blessed, voted_but_not_yet_blessed, committed, last_root_with_interval) = {
                let dest_bless_state = lane_state.dest_bless_worker.unstable_state_read();
                (
                    dest_bless_state.blessed_roots.len(),
                    dest_bless_state.my_voted_but_not_yet_blessed_roots.len(),
                    dest_bless_state.committed_roots_with_intervals.len(),
                    dest_bless_state
                        .committed_roots_with_intervals
                        .last()
                        .copied(),
                )
            };
            let (executed, max_executed_seq_nr) = {
                let dest_offramp_state = lane_state.dest_offramp_worker.unstable_state_read();
                (
                    dest_offramp_state.successfully_executed_seq_nrs.len(),
                    dest_offramp_state.max_sequence_number,
                )
            };
            let votable = {
                let lane_bless_status = lane_state
                    .lane_bless_status_worker
                    .lane_bless_status
                    .lock()
                    .unwrap()
                    .clone();
                if let Some(lane_bless_status) = lane_bless_status {
                    Some(lane_bless_status.verified_tagged_roots.len())
                } else {
                    None
                }
            };

            let (since_last_checked_offramp, dest_reapable_msg_ids, anomalies) = {
                let state = lane_state.offramp_anomaly_detector.state.read().unwrap();
                (
                    state.last_checked.map(|inst| inst.elapsed()),
                    state.seq_nrs_with_reapable_msg_ids.len(),
                    state.anomalies.len(),
                )
            };

            info!(
                ?first_msg,
                ?last_msg,
                ?last_root_with_interval,
                ?max_executed_seq_nr,
                executed,
                ?since_last_checked_offramp,
                dest_reapable_msg_ids,
                anomalies,
                blessed,
                voted_but_not_yet_blessed,
                committed,
                votable,
                "{lane_id} status",
            );
        }
    }

    pub fn monitor(&mut self) -> Result<MonitorResult> {
        let mut status_reported_at: Option<Instant> = None;
        loop {
            let finished_workers_and_results = self.shutdown_handles.finished_workers_and_results();

            self.curse_beacon.update();

            if self.curse_beacon.is_cursed() {
            } else if !finished_workers_and_results.is_empty() {
                return Ok(MonitorResult::WorkersDied(finished_workers_and_results));
            } else {
                let new_onchain_configs = get_onchain_configs(&self.config, &self.chain_states);
                for (&chain_name, versioned_onchain_config) in new_onchain_configs.iter() {
                    let init_versioned_onchain_config = self
                        .onchain_configs
                        .get(&chain_name)
                        .ok_or_else(|| anyhow!("onchain config not found: {chain_name}"))?;
                    if init_versioned_onchain_config != versioned_onchain_config {
                        info!(
                            ?chain_name,
                            ?versioned_onchain_config,
                            ?init_versioned_onchain_config,
                            "afn {chain_name} config changed"
                        );
                        return Ok(MonitorResult::OnchainConfigChanged);
                    }
                }
            }

            if status_reported_at
                .map(|t| t.elapsed() > crate::config::MIN_STATUS_REPORT_INTERVAL)
                .unwrap_or(true)
            {
                self.print_status_report();
                status_reported_at.replace(Instant::now());
            }
            thread::sleep(crate::config::STATE_MONITOR_INTERVAL);
        }
    }
}
