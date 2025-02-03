#![allow(dead_code)]
#![allow(clippy::nonminimal_bool)]
#![allow(clippy::empty_line_after_doc_comments)]

use std::{
    any::type_name,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fmt::Debug,
    io::{Read, Write},
    net::{Shutdown, TcpStream},
    sync::Arc,
    time::Duration,
};

use anyhow::{anyhow, Context as _, Result};
use argh::FromArgs;
use crossbeam_channel::{after, select, tick, Receiver, Sender};
use defer::defer;
use miniabi::{abi_decode::AbiDecode, abi_encode::AbiEncode, types::Value};
use openssl::sha::{sha256, Sha256};

use afn::{
    chain_status::ChainStatus,
    chain_status_worker::{ChainStatusProvider as _, ChainStatusWorker},
    chains::ChainSelector,
    common::ChainName,
    config::ChainStability,
};
use afn::{
    chains::chain_selector,
    config::find_duplicate,
    metrics::{join_prometheus_grouped_lines_with_spacing, ChainStatusMetrics, MetricsFileWorker},
};
use afn::{common::ContractCall, common::EncodeContractCall};
use minieth::{
    bytes::{Address, Bytes32},
    keccak::keccak256,
    rpc::Rpc,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use afn2::contracts::prelude::*;
use afn2::contracts::{
    capabilities_registry as capability_registry_contract, ccip_home as ccip_home_contract,
    ccip_home::CcipOcrConfig, rmn_home as rmn_contract,
    rmn_home::FullConfig as RmnHomeContractConfig,
};
use afn2::{cesm as source_bless_cesm, limits::new_stream_limits};
use afn2::{
    config_digest::{prefix_config_digest, ConfigDigest, ConfigDigestPrefix, WithConfigDigest},
    limits::NewStreamLimits,
};

use afn2::keystore2::*;
use afn2::offchain::*;
use afn2::ragep2p::{Bootstrapper, PeerId};
use afn2::rageproxy::*;
use afn2::signing::*;

trait CcipHomeContractConfigTracker: Send + Sync + 'static {
    fn active_configs(&self) -> Result<Vec<WithConfigDigest<CcipOcrConfig>>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct LaneId {
    source_chain_selector: ChainSelector,
    onramp_address: Address,
    dest_chain_selector: ChainSelector,
    offramp_address: Address,
}

impl From<LaneId> for LaneSource {
    fn from(lane_id: LaneId) -> Self {
        LaneSource {
            source_chain_selector: lane_id.source_chain_selector,
            onramp_address: lane_id.onramp_address,
        }
    }
}

trait OnRampReader {
    /// returns the merkle root of messages with sequence number within the closed interval, for the particular lane
    fn merkle_root(&self, lane_id: &LaneId, closed_interval: ClosedInterval) -> Option<Bytes32>;
}

#[derive(Clone, Debug)]
struct ContractCaller {
    rpc: Arc<Rpc>,
}

impl ContractCaller {
    fn new(rpc: Arc<Rpc>) -> Self {
        Self { rpc }
    }

    fn call<Call: ContractCall + ContractCallReturnValue>(
        &self,
        contract_address: Address,
        call: Call,
    ) -> Result<Call::ReturnValue> {
        let ret_data = self
            .rpc
            .call(contract_address, call.encode_contract_call())
            .with_context(|| anyhow!("while calling contract at {}", contract_address))?;
        Call::ReturnValue::abi_decode(ret_data).with_context(|| {
            anyhow!(
                "while abi decoding type {} from contract {}",
                type_name::<Call::ReturnValue>(),
                contract_address
            )
        })
    }
}

pub fn ccip_capability_id() -> capability_registry_contract::CapabilityId {
    capability_registry_contract::CapabilityId {
        labelled_name: "ccip".to_string(),
        version: "v1.0.0".to_string(),
    }
}

struct CcipHomeContractConfigTrackerCalling {
    chain_id: u64,
    contract_caller: ContractCaller,
    capabilities_registry_address: Address,
    ccip_home_address: Address,
}

impl CcipHomeContractConfigTracker for CcipHomeContractConfigTrackerCalling {
    fn active_configs(&self) -> Result<Vec<WithConfigDigest<CcipOcrConfig>>> {
        let all_don_infos = self.contract_caller.call(
            self.capabilities_registry_address,
            capability_registry_contract::GetDons,
        )?;
        let ccip_don_infos = all_don_infos.into_iter().filter(|don_info| {
            don_info
                .capability_configurations
                .iter()
                .any(|config| config.capability_id == ccip_capability_id().hashed())
        });
        let ccip_don_ids: Vec<u32> = ccip_don_infos.map(|don_info| don_info.id).collect();

        let ccip_ocr_configs_per_chain = ccip_don_ids
            .into_iter()
            .map(|don_id| -> Result<Vec<WithConfigDigest<CcipOcrConfig>>> {
                let (a, b) = self.contract_caller.call(
                    self.ccip_home_address,
                    ccip_home_contract::GetAllConfigs {
                        don_id,
                        plugin_type: ccip_home_contract::OcrPluginType::Commit,
                    },
                )?;
                let aux = ccip_home_contract::CheckConfigDigestAuxInfo {
                    chain_id: self.chain_id,
                    contract: self.ccip_home_address,
                    don_id,
                };
                Ok([a, b]
                    .into_iter()
                    .filter_map(|versioned_config| {
                        versioned_config
                            .clone()
                            .check_config_digest_and_convert(aux)
                            .map_err(|error| {
                                tracing::warn!(
                                    ?versioned_config,
                                    ?aux,
                                    ?error,
                                    "skipping invalid CcipHome config"
                                );
                            })
                            .ok()
                    })
                    .flatten()
                    .collect())
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(ccip_ocr_configs_per_chain
            .into_iter()
            .flatten()
            .collect::<Vec<_>>())
    }
}

trait RmnHomeContractConfigTracker: Send + Sync + 'static {
    fn active_configs(&self) -> Result<Vec<WithConfigDigest<RmnHomeContractConfig>>>;
}

struct RmnHomeContractConfigTrackerCalling {
    chain_id: u64,
    contract_caller: ContractCaller,
    rmn_home_address: Address,
}

impl RmnHomeContractConfigTracker for RmnHomeContractConfigTrackerCalling {
    fn active_configs(&self) -> Result<Vec<WithConfigDigest<RmnHomeContractConfig>>> {
        let (a, b) = self
            .contract_caller
            .call(self.rmn_home_address, rmn_contract::GetAllConfigs)?;

        let aux = rmn_contract::CheckConfigDigestAuxInfo {
            chain_id: self.chain_id,
            contract: self.rmn_home_address,
        };

        Ok([a, b]
            .into_iter()
            .filter_map(|versioned_config| {
                versioned_config
                    .clone()
                    .check_config_digest_and_convert(aux)
                    .map_err(|error| {
                        tracing::warn!(
                            ?versioned_config,
                            ?aux,
                            ?error,
                            "skipping invalid RmnHome config"
                        );
                    })
                    .ok()
            })
            .flatten()
            .collect())
    }
}

#[derive(Debug, Clone)]
struct MultiCesm {
    s_requests: Sender<(LaneId, ClosedInterval)>,
    r_responses: Receiver<Option<Bytes32>>,
    s_metrics_requests: Sender<()>,
    r_metrics_responses: Receiver<Vec<String>>,
    shutdown_handle: Arc<afn::worker::ShutdownHandle>,
}

impl MultiCesm {
    fn spawn(
        ctx: Context,
        rpcs: HashMap<ChainName, Arc<Rpc>>,
        chain_params: Vec<SharedChainParamsConfig>,
        remote_chains: Vec<SharedRemoteChainConfig>,
    ) -> Self {
        let (s_requests, r_requests) = crossbeam_channel::bounded(0);
        let (s_responses, r_responses) = crossbeam_channel::bounded(0);

        let (s_metrics_requests, r_metrics_requests) = crossbeam_channel::bounded(0);
        let (s_metrics_responses, r_metrics_responses) = crossbeam_channel::bounded(0);

        let join_handle = std::thread::spawn(move || {
            run_multi_cesm(
                ctx,
                rpcs,
                chain_params,
                remote_chains,
                (r_requests, s_responses),
                (r_metrics_requests, s_metrics_responses),
            )
        });

        let shutdown_handle = Arc::new(afn::worker::ShutdownHandle::new(
            afn::worker::Context::new(),
            "MultiCesm".to_string(),
            join_handle,
        ));

        Self {
            s_requests,
            r_responses,
            s_metrics_requests,
            r_metrics_responses,
            shutdown_handle,
        }
    }
}

impl MultiCesm {
    fn get_prometheus_file_lines(&self) -> Vec<String> {
        self.s_metrics_requests.send(()).ok();
        self.r_metrics_responses.recv().unwrap_or_default()
    }
}

impl OnRampReader for MultiCesm {
    fn merkle_root(&self, lane_id: &LaneId, closed_interval: ClosedInterval) -> Option<Bytes32> {
        self.s_requests.send((*lane_id, closed_interval)).ok()?;
        self.r_responses.recv().ok()?
    }
}

fn run_multi_cesm(
    ctx: Context,
    rpcs: HashMap<ChainName, Arc<Rpc>>,
    chain_params: Vec<SharedChainParamsConfig>,
    remote_chains: Vec<SharedRemoteChainConfig>,
    (r_requests, s_responses): (Receiver<(LaneId, ClosedInterval)>, Sender<Option<Bytes32>>),
    (r_metrics_requests, s_metrics_responses): (Receiver<()>, Sender<Vec<String>>),
) -> Result<()> {
    struct ManagedCesm {
        cesm: crate::source_bless_cesm::SourceBlessWorker,
        shutdown_handle: afn::worker::ShutdownHandle,
    }

    struct ManagedChainStatusWorker {
        chain_status_worker: Arc<ChainStatusWorker>,
        shutdown_handle: afn::worker::ShutdownHandle,
    }

    let worker_context = afn::worker::Context::new();

    let worker_context_cancel = Arc::clone(&worker_context);
    defer!(worker_context_cancel.cancel());

    let mut chain_status_metrics = ChainStatusMetrics::new();

    let managed_chain_status_workers: HashMap<ChainSelector, ManagedChainStatusWorker> =
        chain_params
            .iter()
            .copied()
            .map(
                |SharedChainParamsConfig { name, stability }| -> anyhow::Result<_> {
                    let rpc = rpcs.get(&name).context("unexpectedly missing rpc")?.clone();

                    let chain_status_metrics_handle = chain_status_metrics.make_handle(name);
                    let (chain_status_worker, shutdown_handle) = ChainStatusWorker::spawn(
                        &worker_context,
                        name,
                        afn::config::CHAIN_STATUS_WORKER_POLL_INTERVAL,
                        afn::chain_state::make_chain_status_updater(rpc, stability),
                        chain_status_metrics_handle,
                    );
                    Ok((
                        chain_selector(name),
                        ManagedChainStatusWorker {
                            chain_status_worker: Arc::new(chain_status_worker),
                            shutdown_handle,
                        },
                    ))
                },
            )
            .collect::<anyhow::Result<HashMap<_, _>>>()
            .context("while initializing chain status workers")?;

    let managed_cesms: HashMap<LaneSource, ManagedCesm> = remote_chains
        .iter()
        .copied()
        .map(
            |SharedRemoteChainConfig {
                 name,
                 on_ramp_start_block_number,
                 on_ramp,
                 ..
             }| -> anyhow::Result<_> {
                let lane_source = LaneSource {
                    source_chain_selector: chain_selector(name),
                    onramp_address: on_ramp,
                };

                let rpc = rpcs.get(&name).context("unexpectedly missing rpc")?.clone();
                let stability = chain_params
                    .iter()
                    .find(|c| c.name == name)
                    .context("unexpectedly missing chain params")?
                    .stability;
                let chain_status_worker = managed_chain_status_workers
                    .get(&lane_source.source_chain_selector)
                    .context("unexpectedly missing chain status worker")?
                    .chain_status_worker
                    .clone();

                let (source_bless_worker, shutdown_handle) =
                    crate::source_bless_cesm::SourceBlessWorker::spawn(
                        &worker_context,
                        &format!("SourceBlessWorker({},{})", name, on_ramp),
                        afn::config::CESM_POLL_INTERVAL,
                        afn::contract_event_state_machine::ContractEventStateMachine::new(
                            Duration::ZERO,
                            on_ramp_start_block_number,
                            vec![lane_source.onramp_address],
                            crate::source_bless_cesm::SourceBlessState::default(),
                            crate::source_bless_cesm::SourceBlessEventFilter::default(),
                        ),
                        name,
                        rpc,
                        chain_status_worker,
                        stability,
                    );

                tracing::info!(%name, %on_ramp, %on_ramp_start_block_number, "MultiCesm: spawned cesm");

                Ok((
                    lane_source,
                    ManagedCesm {
                        cesm: source_bless_worker,
                        shutdown_handle,
                    },
                ))
            },
        )
        .collect::<anyhow::Result<HashMap<_, _>>>()
        .context("while initializing cesms")?;

    let mut lane_ids: HashSet<LaneId> = HashSet::new();
    for source in &remote_chains {
        for dest in &remote_chains {
            if source.connected_component == dest.connected_component && source.name != dest.name {
                lane_ids.insert(LaneId {
                    source_chain_selector: chain_selector(source.name),
                    onramp_address: source.on_ramp,
                    dest_chain_selector: chain_selector(dest.name),
                    offramp_address: dest.off_ramp,
                });
            }
        }
    }

    tracing::info!("MultiCesm: started");

    let ctx_done = ctx.done();

    loop {
        select! {
            recv(ctx_done) -> _ => return Ok(()),
            recv(r_metrics_requests) -> _ => {
                s_metrics_responses.send(
                    join_prometheus_grouped_lines_with_spacing(vec![
                        chain_status_metrics.get_prometheus_file_lines(),
                        worker_context.worker_metrics.lock().unwrap().get_prometheus_file_lines(),
                    ])
                )?;
            }
            recv(r_requests) -> req => {
                let (lane_id, closed_interval) = req?;
                let lane_source = LaneSource::from(lane_id);
                let resp = if lane_ids.contains(&lane_id) {
                    let cesm_output = if let Some(ManagedCesm{cesm, ..}) = managed_cesms.get(&lane_source) {
                        cesm.stable_state_read().merkle_root(lane_id.dest_chain_selector, closed_interval)
                    } else {
                        None
                    };

                    if let Some(ChainStatus::Good{..}) = managed_chain_status_workers
                        .get(&lane_source.source_chain_selector)
                        .and_then(|ManagedChainStatusWorker{chain_status_worker, ..}| chain_status_worker.latest_chain_status()) {
                        cesm_output
                    } else {
                        None
                    }
                } else {
                    None
                };
                s_responses.send(resp)?
            }
        }
    }
}

///////////////////////
/// config tracking ///
///////////////////////

fn run_with_contract_config<ContractConfig: Eq + Clone + Send + Sync + 'static>(
    ctx: Context,
    get_contract_config_interval: Duration,
    get_contract_config: impl Fn() -> Result<ContractConfig> + Send + 'static,
    run_fn_retry_interval: Duration,
    run_fn: impl Fn(Context, ContractConfig) -> Result<()> + Send + Sync + 'static,
) {
    let run_fn = Arc::new(run_fn);

    let mut latest_config: Option<ContractConfig> = None;

    let ctx_done = ctx.done();
    let initial_ticker = after(Duration::ZERO);
    let periodic_ticker = tick(get_contract_config_interval);

    let mut run_join_handle = std::thread::spawn(|| {});

    #[allow(unused_assignments)]
    let (mut run_ctx, mut run_ctx_handle) = Context::new();

    loop {
        select! {
             recv(ctx_done) -> _ => break,
             recv(initial_ticker) -> _ => {},
             recv(periodic_ticker) -> _ => {},
        }

        match get_contract_config() {
            Ok(new_config) => {
                if Some(&new_config) != latest_config.as_ref() {
                    latest_config = Some(new_config.clone());
                } else {
                    continue;
                }

                tracing::info!("stopping prior run...");

                drop(run_ctx_handle);
                (run_ctx, run_ctx_handle) = Context::new();

                if let Err(error) = run_join_handle.join() {
                    tracing::warn!(?error, "prior run panicked");
                }

                tracing::info!("starting new run...");
                run_join_handle = std::thread::spawn({
                    let run_ctx = run_ctx.clone();
                    let new_config = new_config.clone();
                    let run_fn = run_fn.clone();
                    let run_ctx_done = run_ctx.done();
                    let ctx_done = ctx.done();
                    move || loop {
                        if let Err(error) = run_fn(run_ctx.clone(), new_config.clone()) {
                            tracing::warn!(?error, "run failed");
                        }
                        select! {
                            recv(ctx_done) -> _ => break,
                            recv(run_ctx_done) -> _ => break,
                            recv(after(run_fn_retry_interval)) -> _ => {
                                tracing::warn!("retrying run after error...");
                            },
                        }
                    }
                });
            }
            Err(error) => {
                tracing::warn!(?error, "error while fetching contract config, will retry");
            }
        }
    }

    drop(run_ctx_handle);
    let _ = run_join_handle.join();
}

//////////////////////
/// onchain report ///
//////////////////////

#[derive(Clone, Debug)]
struct OnchainReport {
    dest_chain_id: u64,
    dest_chain_selector: ChainSelector,
    rmn_remote_contract_address: Address,
    offramp_address: Address,
    rmn_home_contract_config_digest: Bytes32,
    dest_lane_updates: Vec<FixedDestLaneUpdate>,
}

impl From<OnchainReport> for Value {
    fn from(o: OnchainReport) -> Self {
        (
            o.dest_chain_id,
            o.dest_chain_selector,
            o.rmn_remote_contract_address,
            o.offramp_address,
            o.rmn_home_contract_config_digest,
            o.dest_lane_updates
                .into_iter()
                .map(|dlu| {
                    (
                        dlu.lane_source.source_chain_selector,
                        dlu.lane_source.onramp_address.abi_encode(),
                        dlu.closed_interval.min_msg_nr,
                        dlu.closed_interval.max_msg_nr,
                        dlu.root,
                    )
                })
                .collect::<Vec<_>>(),
        )
            .into()
    }
}

fn onchain_report_signed_hash(report: OnchainReport) -> Bytes32 {
    keccak256(&(Bytes32::from(keccak256(b"RMN_V1_6_ANY2EVM_REPORT")), report).abi_encode()).into()
}

//////////////////////

fn combo_config_digest(
    ccip_ocr_config_digest: ConfigDigest,
    rmn_home_config_digest: ConfigDigest,
) -> ConfigDigest {
    let mut h = Sha256::new();
    h.update(ccip_ocr_config_digest.as_slice());
    h.update(rmn_home_config_digest.as_slice());
    prefix_config_digest(ConfigDigestPrefix::CCIPMultiRoleRMNCombo, h.finish().into())
}

fn observation_signed_hash(o: &Observation) -> Bytes32 {
    let encoded_observation = o.clone().encode_to_vec();
    let mut h = Sha256::new();
    h.update("chainlink ccip 1.6 rmn observation".as_bytes());
    let observation_hash = sha256(&encoded_observation);
    h.update(observation_hash.as_slice());
    h.finish().into()
}

fn handle_observation_request(
    request: ObservationRequest,
    rmn_home_contract_config: &WithConfigDigest<RmnHomeContractConfig>,
    keystore: &dyn Keystore,
    onramp_reader: &dyn OnRampReader,
) -> Result<SignedObservation> {
    let our_rmn_node_index = rmn_home_contract_config
        .config
        .static_config
        .nodes
        .iter()
        .position(|node| node.offchain_public_key == keystore.as_public().offchain_public_key)
        .context("our node not found in RMN config")?;
    let rmn_source_chains = &rmn_home_contract_config.config.dynamic_config.source_chains;

    {
        let dest_chain_selector = request.lane_dest.dest_chain_selector;
        if request.fixed_dest_lane_update_requests.len() > rmn_source_chains.len() {
            return Err(anyhow!(
                "too many fixed dest lane update requests (max: {}, received: {})",
                rmn_source_chains.len(),
                request.fixed_dest_lane_update_requests.len()
            ));
        }

        let mut last_seen_source_chain_selector = None;
        for fdlu in &request.fixed_dest_lane_update_requests {
            let source_chain_selector = fdlu.lane_source.source_chain_selector;
            if let Some(last_seen_source_chain_selector) = last_seen_source_chain_selector {
                if last_seen_source_chain_selector >= source_chain_selector {
                    return Err(anyhow!("source chain selectors not strictly increasing"));
                }
            }
            last_seen_source_chain_selector = Some(source_chain_selector);

            let rmn_source_chain_config = rmn_source_chains
                .iter()
                .find(|sc| sc.chain_selector == source_chain_selector);
            if let Some(rmn_source_chain_config) = rmn_source_chain_config {
                if !rmn_source_chain_config
                    .observer_nodes_bitmap
                    .contains(our_rmn_node_index)
                {
                    return Err(anyhow!(
                        "our node does not support source chain {}",
                        source_chain_selector
                    ));
                }
            } else {
                return Err(anyhow!("source chain not found in RMN config"));
            }

            if source_chain_selector == dest_chain_selector {
                return Err(anyhow!("source chain and dest chain are the same"));
            }
        }
    }

    let fixed_dest_lane_updates = request
        .fixed_dest_lane_update_requests
        .into_iter()
        .flat_map(|req| {
            let root = onramp_reader.merkle_root(
                &LaneId {
                    source_chain_selector: req.lane_source.source_chain_selector,
                    onramp_address: req.lane_source.onramp_address,
                    dest_chain_selector: request.lane_dest.dest_chain_selector,
                    offramp_address: request.lane_dest.offramp_address,
                },
                req.closed_interval,
            )?;
            Some(FixedDestLaneUpdate {
                lane_source: req.lane_source,
                closed_interval: req.closed_interval,
                root,
            })
        })
        .collect::<Vec<_>>();
    let observation = Observation {
        rmn_home_contract_config_digest: rmn_home_contract_config.config_digest,
        lane_dest: request.lane_dest,
        fixed_dest_lane_updates,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs(),
    };
    let signature = keystore.sign_offchain(&observation_signed_hash(&observation));
    Ok(SignedObservation {
        observation,
        signature,
    })
}

fn handle_report_signature_request(
    request: ReportSignatureRequest,
    remote_chains: &[SharedRemoteChainConfig],
    rmn_home_contract_config: &WithConfigDigest<RmnHomeContractConfig>,
    keystore: &dyn Keystore,
) -> Result<ReportSignature> {
    let mut last_seen_signer_node_index = None;
    let mut last_seen_lane_dest = None;
    let WithConfigDigest {
        config_digest,
        config,
    } = rmn_home_contract_config;

    let mut observers_by_fdlu: BTreeMap<FixedDestLaneUpdate, BTreeSet<u32>> = BTreeMap::new();
    for aso in &request.attributed_signed_observations {
        let Some(node) = config
            .static_config
            .nodes
            .get(aso.signer_node_index as usize)
        else {
            return Err(anyhow!("signer_node_index out of bounds"));
        };

        if let Some(last_seen_signer_node_index) = last_seen_signer_node_index {
            if !(last_seen_signer_node_index < aso.signer_node_index) {
                return Err(anyhow!("signer_node_index not strictly increasing"));
            }
        }
        last_seen_signer_node_index = Some(aso.signer_node_index);

        let lane_dest = aso.signed_observation.observation.lane_dest;
        if let Some(last_seen_lane_dest) = last_seen_lane_dest {
            if last_seen_lane_dest != lane_dest {
                return Err(anyhow!("lane_dest mismatch between observations"));
            }
        }
        last_seen_lane_dest = Some(lane_dest);

        if aso
            .signed_observation
            .observation
            .rmn_home_contract_config_digest
            != *config_digest
        {
            return Err(anyhow!("unexpected config digest in observation"));
        }

        verify_ed25519(
            &node.offchain_public_key,
            &observation_signed_hash(&aso.signed_observation.observation),
            &aso.signed_observation.signature,
        )
        .context("invalid observation signature")?;

        let mut last_seen_source_chain_selector = None;
        for fdlu in &aso.signed_observation.observation.fixed_dest_lane_updates {
            let source_chain_selector = fdlu.lane_source.source_chain_selector;
            let dest_chain_selector = lane_dest.dest_chain_selector;
            if source_chain_selector == dest_chain_selector {
                return Err(anyhow!("source chain and dest chain are the same"));
            }

            if let Some(last_seen_source_chain_selector) = last_seen_source_chain_selector {
                if last_seen_source_chain_selector >= source_chain_selector {
                    return Err(anyhow!("source chain selectors not strictly increasing"));
                }
            }
            last_seen_source_chain_selector = Some(source_chain_selector);

            observers_by_fdlu
                .entry(*fdlu)
                .or_default()
                .insert(aso.signer_node_index);
        }
    }

    let mut blessed_fdlus: Vec<FixedDestLaneUpdate> = Vec::new();

    for (fixed_dest_lane_update, observers) in observers_by_fdlu {
        let Some(source_chain_rmn_config) = config.dynamic_config.source_chains.iter().find(|sc| {
            sc.chain_selector == fixed_dest_lane_update.lane_source.source_chain_selector
        }) else {
            return Err(anyhow!("source chain not found in RMN config"));
        };

        for observer in &observers {
            if !source_chain_rmn_config
                .observer_nodes_bitmap
                .contains(*observer as usize)
            {
                return Err(anyhow!("observer not in source chain's bitmap"));
            }
        }

        if observers.len() <= source_chain_rmn_config.f as usize {
            continue;
        }

        if let Some(additional_fixed_dest_lane_update) = blessed_fdlus.iter().find(|bfdlu| {
            bfdlu.lane_source.source_chain_selector
                == fixed_dest_lane_update.lane_source.source_chain_selector
        }) {
            return Err(anyhow!(
                "multiple blessed lane updates for the same source chain: [{:?}, {:?}]",
                additional_fixed_dest_lane_update,
                fixed_dest_lane_update
            ));
        }

        blessed_fdlus.push(fixed_dest_lane_update);
    }

    if let Some(last_seen_lane_dest) = last_seen_lane_dest {
        if last_seen_lane_dest != request.context.lane_dest {
            return Err(anyhow!(
                "lane_dest mismatch between observations and report context"
            ));
        }
    } else {
        return Err(anyhow!("no observations"));
    }

    let context_ok = remote_chains.iter().any(|rc| {
        let ReportContext {
            evm_dest_chain_id,
            rmn_remote_contract_address,
            rmn_home_contract_config_digest,
            lane_dest:
                LaneDest {
                    dest_chain_selector,
                    offramp_address,
                },
        } = request.context;
        rc.name.chain_id() == evm_dest_chain_id
            && chain_selector(rc.name) == dest_chain_selector
            && rc.off_ramp == offramp_address
            && rc.rmn_remote == rmn_remote_contract_address
            && rmn_home_contract_config_digest == *config_digest
    });
    if !context_ok {
        return Err(anyhow!("unexpected request context {:?}", request.context));
    }

    let report = OnchainReport {
        dest_chain_id: request.context.evm_dest_chain_id,
        dest_chain_selector: request.context.lane_dest.dest_chain_selector,
        rmn_remote_contract_address: request.context.rmn_remote_contract_address,
        offramp_address: request.context.lane_dest.offramp_address,
        rmn_home_contract_config_digest: *config_digest,
        dest_lane_updates: blessed_fdlus,
    };

    let signature = keystore.sign_evm_onchain(&onchain_report_signed_hash(report));
    Ok(ReportSignature { signature })
}

fn handle_request(
    request: Request,
    remote_chains: &[SharedRemoteChainConfig],
    rmn_home_contract_config: &WithConfigDigest<RmnHomeContractConfig>,
    keystore: &dyn Keystore,
    onramp_reader: &dyn OnRampReader,
) -> Result<Response> {
    Ok(match request {
        Request::ObservationRequest(request) => Response::SignedObservation(
            handle_observation_request(request, rmn_home_contract_config, keystore, onramp_reader)?,
        ),
        Request::ReportSignatureRequest(request) => {
            Response::ReportSignature(handle_report_signature_request(
                request,
                remote_chains,
                rmn_home_contract_config,
                keystore,
            )?)
        }
    })
}

fn run_with_rageproxy_conn(
    ctx: Context,
    conn: TcpStream,
    bootstrappers: Vec<Bootstrapper>,
    remote_chains: Vec<SharedRemoteChainConfig>,
    keystore: &dyn Keystore,
    onramp_reader: &dyn OnRampReader,
    (mut ccip_ocr_configs, mut rmn_home_configs): (
        Vec<WithConfigDigest<CcipOcrConfig>>,
        Vec<WithConfigDigest<RmnHomeContractConfig>>,
    ),
) -> Result<()> {
    let (to_rageproxy, from_rageproxy) = rageproxy_channel(conn);

    struct ConfigCombo {
        ccip_ocr_config: WithConfigDigest<CcipOcrConfig>,
        rmn_home_config: WithConfigDigest<RmnHomeContractConfig>,
    }
    let mut config_combo_by_stream_name: HashMap<String, ConfigCombo> = HashMap::new();

    rmn_home_configs.retain(
        |WithConfigDigest {
             config,
             config_digest,
         }| {
            let we_are_included =
                config.static_config.nodes.iter().any(|node| {
                    node.offchain_public_key == keystore.as_public().offchain_public_key
                });

            if !we_are_included {
                tracing::info!(
                    %config_digest,
                    "we are not included in the rmn home config, skipping"
                );
            }
            we_are_included
        },
    );

    ccip_ocr_configs.retain(
        |WithConfigDigest {
             config,
             config_digest,
         }| {
            if !config.rmn_enabled {
                tracing::info!(
                    %config_digest,
                    "ccip ocr config is not rmn enabled, skipping"
                );
            }
            config.rmn_enabled
        },
    );

    for rmn_home_config in &rmn_home_configs {
        for ccip_ocr_config in &ccip_ocr_configs {
            let NewStreamLimits {
                outgoing_buffer_size,
                incoming_buffer_size,
                max_message_length,
                messages_limit,
                bytes_limit,
            } = match new_stream_limits(
                ccip_ocr_config.config.estimated_round_interval,
                rmn_home_config.config.dynamic_config.source_chains.len(),
                rmn_home_config.config.static_config.nodes.len(),
            ) {
                Ok(new_stream_limits) => new_stream_limits,
                Err(error) => {
                    tracing::warn!(
                        ?error,
                        ?ccip_ocr_config,
                        ?rmn_home_config,
                        "failed to calculate new stream limits, skipping"
                    );
                    continue;
                }
            };

            let combo_config_digest =
                combo_config_digest(ccip_ocr_config.config_digest, rmn_home_config.config_digest);

            let peer_ids: Vec<PeerId> = {
                let mut btree = BTreeSet::new();
                for peer_id in &ccip_ocr_config.config.peer_ids {
                    btree.insert(*peer_id);
                }
                for node in &rmn_home_config.config.static_config.nodes {
                    btree.insert(node.peer_id);
                }
                btree.into_iter().collect()
            };

            let add_group = AddGroup {
                config_digest: combo_config_digest,
                peer_ids,
                bootstrappers: bootstrappers.to_vec(),
            };
            tracing::info!(?add_group, "constructing group...");
            to_rageproxy.send(ProxyTargetToProxy::AddGroup(add_group))?;

            let stream_name = afn2::offchain::stream_name(combo_config_digest);

            config_combo_by_stream_name.insert(
                stream_name.clone(),
                ConfigCombo {
                    ccip_ocr_config: ccip_ocr_config.clone(),
                    rmn_home_config: rmn_home_config.clone(),
                },
            );

            for oracle_peer_id in &ccip_ocr_config.config.peer_ids {
                let new_stream = NewStream {
                    other: *oracle_peer_id,
                    stream_name: stream_name.clone(),
                    outgoing_buffer_size,
                    incoming_buffer_size,
                    max_message_length,
                    messages_limit,
                    bytes_limit,
                };
                tracing::info!(?new_stream, "constructing stream...");
                to_rageproxy.send(ProxyTargetToProxy::NewStream(new_stream))?;
            }
        }
    }

    let ctx_done = ctx.done();

    loop {
        select! {
            recv(ctx_done) -> _ => return Ok(()),
            recv(from_rageproxy) -> rageproxy_msg => {
                let ProxyToProxyTarget::MessageProxyToProxyTarget(MessageProxyToProxyTarget
                    { sender_peer_id, stream_name, payload }) = rageproxy_msg?;

                tracing::trace!(?stream_name, ?sender_peer_id, ?payload, "received ragep2p message");

                let Some(config_combo) = config_combo_by_stream_name.get(&stream_name) else {
                    tracing::warn!(stream_name, "unexpected stream name, dropping");
                    continue
                };

                let expected_senders = &config_combo.ccip_ocr_config.config.peer_ids;
                if !expected_senders.contains(&sender_peer_id) {
                    tracing::warn!(stream_name, %sender_peer_id, ?expected_senders, "sender is not an oracle, dropping");
                    continue;
                }

                let RequestWithId{ request_id, request } = match RequestWithId::decode(&payload) {
                    Ok(request_with_id) => request_with_id,
                    Err(error) => {
                        tracing::warn!(?error, stream_name, %sender_peer_id, "failed to decode request, dropping");
                        continue
                    }
                };

                tracing::trace!(request_id, ?request, "received request");

                match handle_request(request, &remote_chains, &config_combo.rmn_home_config, keystore, onramp_reader) {
                    Err(error) => {
                        tracing::warn!(?error, stream_name, %sender_peer_id, "failed to handle request");
                    },
                    Ok(response) => {
                        tracing::trace!(request_id, ?response, "sending response");

                        to_rageproxy.send(ProxyTargetToProxy::MessageProxyTargetToProxy(
                            MessageProxyTargetToProxy {
                                receiver_peer_id: sender_peer_id,
                                stream_name,
                                payload: ResponseWithId { request_id, response }.encode_to_vec(),
                            }
                        ))?;
                    }
                }
            }
        }
    }
}

fn rageproxy_channel(
    stream: TcpStream,
) -> (Sender<ProxyTargetToProxy>, Receiver<ProxyToProxyTarget>) {
    let (from_proxy_s, from_proxy_r) = crossbeam_channel::bounded(0);
    let mut r_stream = stream.try_clone().unwrap();

    let (ctx, ctx_handle) = context::Context::new();

    std::thread::spawn(move || -> Result<()> {
        tracing::info!("rageproxy conn read loop: started");
        defer!(tracing::info!("rageproxy conn read loop: exited"));
        let shutdown_stream = r_stream.try_clone().unwrap();
        defer!({
            let _ = shutdown_stream.shutdown(Shutdown::Both);
            drop(ctx_handle);
        });
        loop {
            let mut len_buf = [0u8; 4];
            r_stream.read_exact(&mut len_buf)?;
            let len = u32::from_be_bytes(len_buf);
            if len > MAX_RAGEPROXY_INCOMING_MESSAGE_SIZE {
                return Err(anyhow::anyhow!(
                    "rageproxy message too large: {} > {}",
                    len,
                    MAX_RAGEPROXY_INCOMING_MESSAGE_SIZE
                ));
            }
            let mut buf = vec![0u8; len as usize];
            r_stream.read_exact(&mut buf)?;
            let proxy_to_proxy_target = ProxyToProxyTarget::decode(&buf)?;
            from_proxy_s.send(proxy_to_proxy_target)?;
        }
    });

    let (to_proxy_s, to_proxy_r) = crossbeam_channel::bounded::<ProxyTargetToProxy>(0);
    let mut w_stream = stream.try_clone().unwrap();

    std::thread::spawn(move || -> Result<()> {
        tracing::info!("rageproxy conn write loop: started");
        defer!(tracing::info!("rageproxy conn write loop: exited"));
        let shutdown_stream = w_stream.try_clone().unwrap();
        defer!({
            let _ = shutdown_stream.shutdown(Shutdown::Both);
        });
        let ctx_done = ctx.done();
        loop {
            select! {
                recv(ctx_done) -> _ => return Ok(()),
                recv(to_proxy_r) -> msg => {
                    let encoding = msg?.encode_to_vec();
                    let encoding_len = encoding.len() as u32;
                    w_stream.write_all(&encoding_len.to_be_bytes())?;
                    w_stream.write_all(&encoding)?;
                }
            }
        }
    });

    (to_proxy_s, from_proxy_r)
}

//////////////////////////
/// toml configuration ///
//////////////////////////

#[derive(Debug, Clone, Copy, Deserialize)]
struct SharedHomeChainConfig {
    pub name: ChainName,
    pub capabilities_registry: Address,
    pub ccip_home: Address,
    pub rmn_home: Address,
}

#[derive(Debug, Clone, Copy, Deserialize)]
struct SharedRemoteChainConfig {
    #[serde(default)]
    /// All remote chains with the same connected_component are assumed to be connected with each other.
    /// Can bump this number for a new connected component, e.g., when upgrading to new lane versions.
    pub connected_component: u64,

    pub name: ChainName,
    pub on_ramp_start_block_number: u64,
    pub on_ramp: Address,
    pub off_ramp: Address,
    pub rmn_remote: Address,
}

#[derive(Debug, Clone, Copy, Deserialize)]
struct SharedChainParamsConfig {
    pub name: ChainName,
    pub stability: ChainStability,
}

#[derive(Debug, Clone, Deserialize)]
struct SharedNetworkingConfig {
    pub bootstrappers: Vec<Bootstrapper>,
}

#[derive(Debug, Clone, Deserialize)]
struct SharedConfig {
    pub networking: SharedNetworkingConfig,
    pub home_chain: SharedHomeChainConfig,
    pub chain_params: Vec<SharedChainParamsConfig>,
    pub remote_chains: Vec<SharedRemoteChainConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LocalChainConfig {
    pub name: ChainName,
    pub rpc: String,
}

#[derive(Debug, Clone, Deserialize)]
struct LocalNetworkingConfig {
    pub rageproxy: String,
}

#[derive(Debug, Clone, Deserialize)]
struct LocalConfig {
    pub networking: LocalNetworkingConfig,
    pub chains: Vec<LocalChainConfig>,
}

fn sanity_check_offchain_configs(
    shared_config: &SharedConfig,
    local_config: &LocalConfig,
) -> anyhow::Result<()> {
    let chains_that_should_have_rpcs = shared_config
        .remote_chains
        .iter()
        .map(|rc| rc.name)
        .chain(std::iter::once(shared_config.home_chain.name))
        .collect::<HashSet<_>>();
    let chains_that_have_rpcs = local_config
        .chains
        .iter()
        .map(|lc| lc.name)
        .collect::<HashSet<_>>();
    if !chains_that_have_rpcs.is_superset(&chains_that_should_have_rpcs) {
        return Err(anyhow!(
            "missing rpcs for chains: {:?}",
            chains_that_should_have_rpcs.difference(&chains_that_have_rpcs)
        ));
    }

    let chains_that_should_have_chain_params = shared_config
        .remote_chains
        .iter()
        .map(|rc| rc.name)
        .collect::<HashSet<_>>();
    let chains_that_have_chain_params = shared_config
        .chain_params
        .iter()
        .map(|cpc| cpc.name)
        .collect::<HashSet<_>>();
    if !chains_that_have_chain_params.is_superset(&chains_that_should_have_chain_params) {
        return Err(anyhow!(
            "missing chain params for chains: {:?}",
            chains_that_should_have_chain_params.difference(&chains_that_have_chain_params)
        ));
    }

    if let Some(dup) = find_duplicate(&shared_config.chain_params, |cpc| cpc.name) {
        return Err(anyhow!("duplicate chain param: {:?}", dup));
    }

    if let Some(dup) = find_duplicate(&local_config.chains, |lc| lc.name) {
        return Err(anyhow!("duplicate entry in local config: {:?}", dup));
    }

    if let Some(dup) = find_duplicate(&shared_config.remote_chains, |rc| {
        (rc.name, rc.connected_component)
    }) {
        return Err(anyhow!("duplicate remote chain: {:?}", dup));
    }

    if let Some(dup) = find_duplicate(&shared_config.remote_chains, |rc| (rc.name, rc.on_ramp)) {
        return Err(anyhow!("duplicate on-ramp: {:?}", dup));
    }

    Ok(())
}

fn must_env_var(var: &str) -> Result<String> {
    std::env::var(var).with_context(|| anyhow!("missing env var {}", var))
}

fn read_toml_from_env_path<T: DeserializeOwned>(env_var: &str) -> Result<T> {
    let path = must_env_var(env_var)?;
    let contents =
        std::fs::read_to_string(&path).with_context(|| anyhow!("while reading file {}", path))?;
    Ok(toml::from_str(&contents)?)
}

fn load_configs() -> Result<(SharedConfig, LocalConfig)> {
    Ok((
        read_toml_from_env_path("AFN_SHARED_CONFIG")?,
        read_toml_from_env_path("AFN_LOCAL_CONFIG")?,
    ))
}

const RAGEPROXY_CONNECTION_RETRY_INTERVAL: Duration = Duration::from_secs(1);

#[derive(FromArgs)]
/// afn v1.6
struct Cli {
    /// print the version and exit
    #[argh(switch)]
    version: bool,

    /// path to the keystore
    #[argh(option)]
    keystore: Option<String>,

    /// whether to generate a keystore
    #[argh(switch)]
    generate: bool,
}

mod context {

    use crossbeam_channel::{Receiver, Sender};

    #[derive(Debug, Clone)]
    pub struct Context {
        done_r: Receiver<()>,
    }

    pub struct ContextHandle {
        done_s: Sender<()>,
    }

    impl Context {
        pub fn new() -> (Self, ContextHandle) {
            let (done_s, done_r) = crossbeam_channel::bounded(0);
            (Self { done_r }, ContextHandle { done_s })
        }

        pub fn done(&self) -> Receiver<()> {
            self.done_r.clone()
        }
    }
}

use context::*;

fn main() -> Result<()> {
    afn::logging::setup_tracing()?;

    let cli: Cli = argh::from_env();

    println!(
        "RMN v1.6 offchain node version: {}-ALPHA
compatible with the following contract versions:
  974def52d97ee548b7568cf2facbc556dfa0e797
    CapabilitiesRegistry
    CCIPHome
    OnRamp
    OffRamp
    RMNHome
    RMNRemote
ccip capability id: {:?}",
        env!("GIT_HASH"),
        ccip_capability_id(),
    );
    if cli.version {
        return Ok(());
    }

    let keystore_path = cli.keystore.context("need to specify --keystore")?;

    let passphrase = must_env_var("AFN_PASSPHRASE")?;

    if cli.generate {
        let keystore = LocalKeystore::generate();
        keystore
            .encrypt_file(&keystore_path, &passphrase)
            .context("while writing generated keystore")?;
        println!("keystore generated and saved to {}", keystore_path);
        return Ok(());
    }

    let keystore = LocalKeystore::decrypt_file(&keystore_path, &passphrase)?;
    println!("loaded keystore: {:?}", keystore.as_public());

    let (shared_config, local_config) = load_configs()?;
    sanity_check_offchain_configs(&shared_config, &local_config)?;

    let SharedConfig {
        networking: SharedNetworkingConfig { bootstrappers },
        home_chain: home_chain_config,
        chain_params: chain_param_configs,
        remote_chains: remote_chain_configs,
    } = shared_config;
    let LocalConfig {
        networking: LocalNetworkingConfig { rageproxy },
        chains: local_chains,
    } = local_config;

    let rpcs = local_chains
        .into_iter()
        .map(|chain| {
            Ok((
                chain.name,
                Arc::new(
                    Rpc::new(chain.name.chain_id(), &chain.rpc)
                        .with_context(|| anyhow!("during Rpc::new for chain {}", chain.name))?,
                ),
            ))
        })
        .collect::<Result<HashMap<ChainName, _>>>()
        .context("while initializing rpcs")?;

    let home_chain_rpc = Arc::clone(
        rpcs.get(&home_chain_config.name)
            .context("no rpc for home chain")?,
    );

    let (background_ctx, _background_ctx_handle) = Context::new();
    let get_home_contract_configs = move || {
        let home_chain_contract_caller = ContractCaller::new(Arc::clone(&home_chain_rpc));
        let ccip_tracker = CcipHomeContractConfigTrackerCalling {
            chain_id: home_chain_config.name.chain_id(),
            contract_caller: home_chain_contract_caller.clone(),
            capabilities_registry_address: home_chain_config.capabilities_registry,
            ccip_home_address: home_chain_config.ccip_home,
        };
        let rmn_tracker = RmnHomeContractConfigTrackerCalling {
            chain_id: home_chain_config.name.chain_id(),
            contract_caller: home_chain_contract_caller.clone(),
            rmn_home_address: home_chain_config.rmn_home,
        };

        Ok((
            ccip_tracker.active_configs()?,
            rmn_tracker.active_configs()?,
        ))
    };

    let multi_cesm = Arc::new(MultiCesm::spawn(
        background_ctx.clone(),
        rpcs.clone(),
        chain_param_configs,
        remote_chain_configs.clone(),
    ));

    let (_metrics_file_worker, _metrics_file_worker_shutdown_handle) = MetricsFileWorker::spawn(
        afn::worker::Context::new(),
        {
            let multi_cesm = Arc::clone(&multi_cesm);
            move || multi_cesm.get_prometheus_file_lines()
        },
        must_env_var("AFN_METRICS_FILE").ok().map(Into::into),
    )?;

    run_with_contract_config(
        background_ctx.clone(),
        afn::config::CONFIG_DISCOVERY_RETRY_INTERVAL,
        get_home_contract_configs,
        RAGEPROXY_CONNECTION_RETRY_INTERVAL,
        move |run_ctx, onchain_configs| {
            let rageproxy_conn =
                TcpStream::connect(&rageproxy).context("while connecting to rageproxy")?;
            run_with_rageproxy_conn(
                run_ctx,
                rageproxy_conn,
                bootstrappers.clone(),
                remote_chain_configs.clone(),
                &keystore,
                multi_cesm.as_ref(),
                onchain_configs,
            )
        },
    );
    Ok(())
}
