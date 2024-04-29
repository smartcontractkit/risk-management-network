#![allow(clippy::new_without_default)]

use anyhow::{bail, Context, Result};
use core::hash::Hash;
use minieth::{rpc::Rpc, tx_sender::TransactionSenderFeeConfig, u256::U256};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    str::FromStr,
    sync::{Arc, Mutex},
};
use tracing::{debug, info};

const DEFAULT_METRICS_FILE_PATH: &str = "./metrics/rmn-metrics.prom";

use crate::{
    common::{ChainName, LaneId},
    config::{
        SharedChainConfig, GAS_FEE_METRICS_UPDATE_WORKER_POLL_INTERVAL,
        METRICS_FILE_WORKER_POLL_INTERVAL,
    },
    worker::{self, ShutdownHandle},
};

#[derive(Debug, Eq, Hash, PartialEq, Clone)]
pub struct MetricLabel {
    label_name: String,
    label_value: String,
}

impl MetricLabel {
    pub fn new(label_name: &str, label_value: &str) -> Self {
        Self {
            label_name: label_name.to_string(),
            label_value: label_value.to_string(),
        }
    }
    pub fn to_prometheus_format(&self) -> String {
        format!("{}=\"{}\"", self.label_name, self.label_value)
    }
}

#[derive(Debug, Clone)]
struct MetricSample {
    name: String,
    labels: Vec<MetricLabel>,
    value: f64,
}

impl MetricSample {
    pub fn to_prometheus_format_line(&self) -> String {
        let labels_str = {
            if self.labels.is_empty() {
                String::new()
            } else {
                format!(
                    "{{{}}}",
                    self.labels
                        .iter()
                        .map(MetricLabel::to_prometheus_format)
                        .collect::<Vec<String>>()
                        .join(",")
                )
            }
        };
        format!("{}{} {}", self.name, labels_str, self.value)
    }
}
enum MetricType {
    Counter,
    Gauge,
}
impl ToString for MetricType {
    fn to_string(&self) -> String {
        match self {
            Self::Counter => "counter",
            Self::Gauge => "gauge",
        }
        .to_string()
    }
}

fn get_help_and_type_lines_helper(
    metric_name: &str,
    metric_type: MetricType,
    doc_string: &str,
) -> Vec<String> {
    vec![
        format!("# HELP {} {}", metric_name, doc_string,),
        format!("# TYPE {} {}", metric_name, metric_type.to_string(),),
    ]
}

pub trait LabelKey: Hash + Eq {
    fn to_metric_labels(&self) -> Vec<MetricLabel>;
}

impl LabelKey for ChainName {
    fn to_metric_labels(&self) -> Vec<MetricLabel> {
        vec![MetricLabel::new("chain", &self.to_string())]
    }
}
impl LabelKey for LaneId {
    fn to_metric_labels(&self) -> Vec<MetricLabel> {
        vec![
            MetricLabel::new("source_chain", &self.source_chain_name.to_string()),
            MetricLabel::new("dest_chain", &self.dest_chain_name.to_string()),
            MetricLabel::new("lane", &self.name.to_string()),
        ]
    }
}

trait Metric<K: LabelKey> {
    fn new(name: &str, doc_string: &str) -> Self;
    fn help_and_type_lines(&self) -> Vec<String>;
    fn take_samples(&self) -> Vec<MetricSample>;
    fn get_prometheus_file_lines(&self) -> Vec<String> {
        [
            self.help_and_type_lines(),
            self.take_samples()
                .into_iter()
                .map(|sample| sample.to_prometheus_format_line())
                .collect(),
        ]
        .concat()
    }
}

#[derive(Debug)]
pub struct MetricTypeCounter<K: LabelKey> {
    name: String,
    doc_string: String,
    handle_by_key: HashMap<K, MetricTypeCounterHandle>,
}

#[derive(Clone, Debug)]
#[cfg_attr(test, derive(Default))]
pub struct MetricTypeCounterHandle {
    value: Arc<Mutex<f64>>,
}

impl MetricTypeCounterHandle {
    pub fn value(&self) -> f64 {
        *self.value.lock().unwrap()
    }

    pub fn inc(&self) {
        *self.value.lock().unwrap() += 1f64;
    }
}

impl<K: LabelKey> Metric<K> for MetricTypeCounter<K> {
    fn new(name: &str, doc_string: &str) -> Self {
        Self {
            name: name.to_string(),
            doc_string: doc_string.to_string(),
            handle_by_key: HashMap::new(),
        }
    }
    fn help_and_type_lines(&self) -> Vec<String> {
        get_help_and_type_lines_helper(&self.name, MetricType::Counter, &self.doc_string)
    }
    fn take_samples(&self) -> Vec<MetricSample> {
        self.handle_by_key
            .iter()
            .map(|(key, handle)| MetricSample {
                name: self.name.clone(),
                labels: key.to_metric_labels(),
                value: handle.value(),
            })
            .collect()
    }
}

impl<K: LabelKey> MetricTypeCounter<K> {
    pub fn make_handle(&mut self, label_key: K) -> MetricTypeCounterHandle {
        match self.handle_by_key.get(&label_key) {
            Some(handle) => handle.clone(),
            None => {
                let new_handle = MetricTypeCounterHandle {
                    value: Arc::new(Mutex::new(Default::default())),
                };
                self.handle_by_key.insert(label_key, new_handle.clone());
                new_handle
            }
        }
    }
}

#[derive(Debug)]
pub struct MetricTypeGauge<K: LabelKey> {
    name: String,
    doc_string: String,
    handle_by_key: HashMap<K, MetricTypeGaugeHandle>,
}

#[derive(Clone, Debug)]
#[cfg_attr(test, derive(Default))]
pub struct MetricTypeGaugeHandle {
    value: Arc<Mutex<Option<f64>>>,
}

impl MetricTypeGaugeHandle {
    pub fn value(&self) -> Option<f64> {
        *self.value.lock().unwrap()
    }

    pub fn set(&self, new_value: f64) {
        self.value.lock().unwrap().replace(new_value);
    }
}

impl<K: LabelKey> Metric<K> for MetricTypeGauge<K> {
    fn new(name: &str, doc_string: &str) -> Self {
        Self {
            name: name.to_string(),
            doc_string: doc_string.to_string(),
            handle_by_key: HashMap::new(),
        }
    }
    fn help_and_type_lines(&self) -> Vec<String> {
        get_help_and_type_lines_helper(&self.name, MetricType::Gauge, &self.doc_string)
    }
    fn take_samples(&self) -> Vec<MetricSample> {
        self.handle_by_key
            .iter()
            .flat_map(|(key, handle)| {
                Some(MetricSample {
                    name: self.name.clone(),
                    labels: key.to_metric_labels(),
                    value: handle.value()?,
                })
            })
            .collect()
    }
}

impl<K: LabelKey> MetricTypeGauge<K> {
    pub fn make_handle(&mut self, label_key: K) -> MetricTypeGaugeHandle {
        match self.handle_by_key.get(&label_key) {
            Some(handle) => handle.clone(),
            None => {
                let new_handle = MetricTypeGaugeHandle {
                    value: Arc::new(Mutex::new(Default::default())),
                };
                self.handle_by_key.insert(label_key, new_handle.clone());
                new_handle
            }
        }
    }
}

impl LabelKey for () {
    fn to_metric_labels(&self) -> Vec<MetricLabel> {
        Vec::new()
    }
}

fn join_prometheus_grouped_lines_with_spacing(grouped_lines: Vec<Vec<String>>) -> Vec<String> {
    grouped_lines
        .into_iter()
        .enumerate()
        .flat_map(|(idx, v)| {
            if idx > 0 {
                vec![vec![String::new()], v]
            } else {
                vec![v]
            }
        })
        .flatten()
        .collect()
}

pub struct LaneStatusMetrics {
    pub committed_messages: MetricTypeGauge<LaneId>,
    pub blessed_messages: MetricTypeGauge<LaneId>,
    pub executed_messages: MetricTypeGauge<LaneId>,
    pub votable_messages: MetricTypeGauge<LaneId>,
    pub voted_but_not_yet_blessed_messages: MetricTypeGauge<LaneId>,
    pub anomalies: MetricTypeGauge<LaneId>,
}

impl LaneStatusMetrics {
    pub fn new() -> Self {
        let committed_messages = MetricTypeGauge::new(
            "rmn_committed_messages",
            "number of committed CCIP messages",
        );
        let blessed_messages =
            MetricTypeGauge::new("rmn_blessed_messages", "number of blessed CCIP messages");
        let executed_messages =
            MetricTypeGauge::new("rmn_executed_messages", "number of executed CCIP messages");
        let votable_messages = MetricTypeGauge::new(
            "rmn_votable_messages",
            "number of CCIP messages we could vote to bless",
        );
        let voted_but_not_yet_blessed_messages = MetricTypeGauge::new(
            "rmn_voted_but_not_yet_blessed_messages",
            "number of CCIP messages we have voted to bless, but are not blessed yet",
        );
        let anomalies = MetricTypeGauge::new("rmn_anomalies", "number of anomalies detected");
        Self {
            committed_messages,
            blessed_messages,
            executed_messages,
            votable_messages,
            voted_but_not_yet_blessed_messages,
            anomalies,
        }
    }
    fn get_prometheus_file_lines(&self) -> Vec<String> {
        join_prometheus_grouped_lines_with_spacing(vec![
            self.committed_messages.get_prometheus_file_lines(),
            self.blessed_messages.get_prometheus_file_lines(),
            self.executed_messages.get_prometheus_file_lines(),
            self.votable_messages.get_prometheus_file_lines(),
            self.voted_but_not_yet_blessed_messages
                .get_prometheus_file_lines(),
            self.anomalies.get_prometheus_file_lines(),
        ])
    }
    pub fn make_handle(&mut self, lane_id: LaneId) -> LaneStatusMetricsHandle {
        LaneStatusMetricsHandle {
            committed_messages: self.committed_messages.make_handle(lane_id.clone()),
            blessed_messages: self.blessed_messages.make_handle(lane_id.clone()),
            executed_messages: self.executed_messages.make_handle(lane_id.clone()),
            votable_messages: self.votable_messages.make_handle(lane_id.clone()),
            voted_but_not_yet_blessed_messages: self
                .voted_but_not_yet_blessed_messages
                .make_handle(lane_id.clone()),
            anomalies: self.anomalies.make_handle(lane_id.clone()),
        }
    }
}

pub type WorkerId = String;
impl LabelKey for WorkerId {
    fn to_metric_labels(&self) -> Vec<MetricLabel> {
        vec![MetricLabel::new("worker", self)]
    }
}

#[derive(Debug)]
pub struct WorkerMetrics {
    pub last_poll_duration_seconds: MetricTypeGauge<WorkerId>,
    pub next_poll_delay_seconds: MetricTypeGauge<WorkerId>,
    pub worker_errors: MetricTypeCounter<WorkerId>,
    pub consecutive_worker_errors: MetricTypeGauge<WorkerId>,
}
#[derive(Debug)]
pub struct WorkerMetricsHandle {
    pub last_poll_duration_seconds: MetricTypeGaugeHandle,
    pub next_poll_delay_seconds: MetricTypeGaugeHandle,
    pub worker_errors: MetricTypeCounterHandle,
    pub consecutive_worker_errors: MetricTypeGaugeHandle,
}

impl WorkerMetrics {
    pub fn new() -> Self {
        let last_poll_duration_seconds = MetricTypeGauge::new(
            "rmn_last_poll_duration_seconds",
            "duration of last poll in seconds",
        );
        let next_poll_delay_seconds = MetricTypeGauge::new(
            "rmn_next_poll_delay_seconds",
            "delay until next poll in seconds",
        );
        let worker_errors =
            MetricTypeCounter::new("rmn_worker_errors", "number of errors encountered");
        let consecutive_worker_errors = MetricTypeGauge::new(
            "rmn_consecutive_worker_errors",
            "number of consecutive errors encountered",
        );
        Self {
            last_poll_duration_seconds,
            next_poll_delay_seconds,
            worker_errors,
            consecutive_worker_errors,
        }
    }
    fn get_prometheus_file_lines(&self) -> Vec<String> {
        join_prometheus_grouped_lines_with_spacing(vec![
            self.last_poll_duration_seconds.get_prometheus_file_lines(),
            self.next_poll_delay_seconds.get_prometheus_file_lines(),
            self.worker_errors.get_prometheus_file_lines(),
            self.consecutive_worker_errors.get_prometheus_file_lines(),
        ])
    }
    pub fn make_handle(&mut self, worker_id: WorkerId) -> WorkerMetricsHandle {
        WorkerMetricsHandle {
            last_poll_duration_seconds: self
                .last_poll_duration_seconds
                .make_handle(worker_id.clone()),
            next_poll_delay_seconds: self.next_poll_delay_seconds.make_handle(worker_id.clone()),
            worker_errors: self.worker_errors.make_handle(worker_id.clone()),
            consecutive_worker_errors: self
                .consecutive_worker_errors
                .make_handle(worker_id.clone()),
        }
    }
}

pub struct LaneStatusMetricsHandle {
    pub committed_messages: MetricTypeGaugeHandle,
    pub blessed_messages: MetricTypeGaugeHandle,
    pub executed_messages: MetricTypeGaugeHandle,
    pub votable_messages: MetricTypeGaugeHandle,
    pub voted_but_not_yet_blessed_messages: MetricTypeGaugeHandle,
    pub anomalies: MetricTypeGaugeHandle,
}

pub struct ChainStatusMetrics {
    latest_block_number: MetricTypeGauge<ChainName>,
    finalized_block_number: MetricTypeGauge<ChainName>,
    finality_violated: MetricTypeGauge<ChainName>,
}

impl ChainStatusMetrics {
    pub fn new() -> Self {
        let latest_block_number =
            MetricTypeGauge::new("rmn_latest_block_number", "block number of latest block");
        let finalized_block_number = MetricTypeGauge::new(
            "rmn_finalized_block_number",
            "block number of finalized block",
        );
        let finality_violated = MetricTypeGauge::new(
            "rmn_finality_violated",
            "boolean flag (0/1) indicating whether a chain has suffered a finality violation since startup",
        );
        Self {
            latest_block_number,
            finalized_block_number,
            finality_violated,
        }
    }
    fn get_prometheus_file_lines(&self) -> Vec<String> {
        join_prometheus_grouped_lines_with_spacing(vec![
            self.latest_block_number.get_prometheus_file_lines(),
            self.finalized_block_number.get_prometheus_file_lines(),
            self.finality_violated.get_prometheus_file_lines(),
        ])
    }
    pub fn make_handle(&mut self, chain_name: ChainName) -> ChainStatusMetricsHandle {
        ChainStatusMetricsHandle {
            latest_block_number: self.latest_block_number.make_handle(chain_name),
            finalized_block_number: self.finalized_block_number.make_handle(chain_name),
            finality_violated: self.finality_violated.make_handle(chain_name),
        }
    }
}

#[cfg_attr(test, derive(Default))]
pub struct ChainStatusMetricsHandle {
    pub latest_block_number: MetricTypeGaugeHandle,
    pub finalized_block_number: MetricTypeGaugeHandle,
    pub finality_violated: MetricTypeGaugeHandle,
}

pub struct GasFeeMetrics {
    onchain_fee_per_gas_wei: MetricTypeGauge<ChainName>,
    bless_max_fee_per_gas_wei: MetricTypeGauge<ChainName>,
    curse_max_fee_per_gas_wei: MetricTypeGauge<ChainName>,
}

impl GasFeeMetrics {
    pub fn new() -> Self {
        let onchain_fee_per_gas_wei = MetricTypeGauge::new(
            "rmn_onchain_fee_per_gas_wei",
            "fee per gas (in wei) to get a tx included on chain. For EIP1559 chains this is the base fee of latest block. \
            For non-EIP1559 (legacy) chains this is the value returned by eth_gasPrice.",
        );
        let bless_max_fee_per_gas_wei = MetricTypeGauge::new(
            "rmn_bless_max_fee_per_gas_wei",
            "configured max fee (in wei) per gas for vote to bless transactions",
        );
        let curse_max_fee_per_gas_wei = MetricTypeGauge::new(
            "rmn_curse_max_fee_per_gas_wei",
            "configured max fee (in wei) per gas for vote to curse transactions",
        );
        Self {
            onchain_fee_per_gas_wei,
            bless_max_fee_per_gas_wei,
            curse_max_fee_per_gas_wei,
        }
    }
    fn get_prometheus_file_lines(&self) -> Vec<String> {
        join_prometheus_grouped_lines_with_spacing(vec![
            self.onchain_fee_per_gas_wei.get_prometheus_file_lines(),
            self.bless_max_fee_per_gas_wei.get_prometheus_file_lines(),
            self.curse_max_fee_per_gas_wei.get_prometheus_file_lines(),
        ])
    }
    pub fn make_handle(&mut self, chain_name: ChainName) -> GasFeeMetricsHandle {
        GasFeeMetricsHandle {
            onchain_fee_per_gas_wei: self.onchain_fee_per_gas_wei.make_handle(chain_name),
            bless_max_fee_per_gas_wei: self.bless_max_fee_per_gas_wei.make_handle(chain_name),
            curse_max_fee_per_gas_wei: self.curse_max_fee_per_gas_wei.make_handle(chain_name),
        }
    }
}

pub struct GasFeeMetricsHandle {
    pub onchain_fee_per_gas_wei: MetricTypeGaugeHandle,
    pub bless_max_fee_per_gas_wei: MetricTypeGaugeHandle,
    pub curse_max_fee_per_gas_wei: MetricTypeGaugeHandle,
}

pub struct OnchainAndConfigFees {
    pub onchain_fee_per_gas_wei: f64,
    pub bless_max_fee_per_gas_wei: f64,
    pub curse_max_fee_per_gas_wei: f64,
}

pub struct GasFeeMetricsUpdateWorker;

impl GasFeeMetricsUpdateWorker {
    pub fn get_onchain_and_config_fees(
        rpc: &Rpc,
        chain_config: &SharedChainConfig,
    ) -> Result<OnchainAndConfigFees> {
        match (chain_config.bless_fee_config, chain_config.curse_fee_config) {
            (
                TransactionSenderFeeConfig::Eip1559 {
                    max_fee_per_gas: bless_max_fee_per_gas,
                    max_priority_fee_per_gas: _,
                },
                TransactionSenderFeeConfig::Eip1559 {
                    max_fee_per_gas: curse_max_fee_per_gas,
                    max_priority_fee_per_gas: _,
                },
            ) => Ok(OnchainAndConfigFees {
                onchain_fee_per_gas_wei: rpc
                    .get_latest_block_base_fee_per_gas_wei()?
                    .to_f64_lossy(),
                bless_max_fee_per_gas_wei: U256::from(bless_max_fee_per_gas).to_f64_lossy(),
                curse_max_fee_per_gas_wei: U256::from(curse_max_fee_per_gas).to_f64_lossy(),
            }),
            (
                TransactionSenderFeeConfig::Legacy {
                    gas_price: bless_max_gas_price,
                },
                TransactionSenderFeeConfig::Legacy {
                    gas_price: curse_max_gas_price,
                },
            ) => Ok(OnchainAndConfigFees {
                onchain_fee_per_gas_wei: rpc.get_gas_price_wei()?.to_f64_lossy(),
                bless_max_fee_per_gas_wei: U256::from(bless_max_gas_price).to_f64_lossy(),
                curse_max_fee_per_gas_wei: U256::from(curse_max_gas_price).to_f64_lossy(),
            }),
            _ => {
                bail!(
                    "inconsistent bless and curse chain config type, bless={:?} curse={:?}",
                    chain_config.bless_fee_config,
                    chain_config.curse_fee_config
                )
            }
        }
    }

    fn wei_to_human_friendly_string(wei: f64) -> String {
        let gwei = wei / 1e9;
        if gwei >= 0.01 {
            format!("{} Gwei", (gwei * 100.0).round() / 100.0)
        } else {
            format!("{} Wei", wei)
        }
    }

    pub fn spawn(
        ctx: Arc<worker::Context>,
        chain_config: SharedChainConfig,
        rpc: Arc<Rpc>,
        gas_fee_metrics_handle: GasFeeMetricsHandle,
    ) -> Result<(Self, ShutdownHandle)> {
        let worker_name = format!("GasFeeMetricsUpdateWorker({})", chain_config.name);
        let handle = {
            ctx.spawn_repeat(
                worker_name.clone(),
                GAS_FEE_METRICS_UPDATE_WORKER_POLL_INTERVAL,
                move |_ctx: &worker::Context| -> Result<()> {
                    let OnchainAndConfigFees {
                        onchain_fee_per_gas_wei,
                        bless_max_fee_per_gas_wei,
                        curse_max_fee_per_gas_wei,
                    } = Self::get_onchain_and_config_fees(&rpc, &chain_config)?;
                    gas_fee_metrics_handle
                        .onchain_fee_per_gas_wei
                        .set(onchain_fee_per_gas_wei);
                    gas_fee_metrics_handle
                        .bless_max_fee_per_gas_wei
                        .set(bless_max_fee_per_gas_wei);
                    gas_fee_metrics_handle
                        .curse_max_fee_per_gas_wei
                        .set(curse_max_fee_per_gas_wei);

                    info!(
                        onchain_fee_per_gas =
                            Self::wei_to_human_friendly_string(onchain_fee_per_gas_wei),
                        bless_max_fee_per_gas =
                            Self::wei_to_human_friendly_string(bless_max_fee_per_gas_wei),
                        curse_max_fee_per_gas =
                            Self::wei_to_human_friendly_string(curse_max_fee_per_gas_wei),
                        "{worker_name}: fee status"
                    );
                    Ok(())
                },
            )
        };
        Ok((Self, handle))
    }
}

pub struct Metrics {
    pub uptime_seconds: MetricTypeGauge<()>,
    pub disabled_chains: MetricTypeGauge<()>,
    pub sent_bless_txs: MetricTypeCounter<ChainName>,
    pub gas_fee_metrics: GasFeeMetrics,
    pub chain_status_metrics: ChainStatusMetrics,
    pub lane_status_metrics: LaneStatusMetrics,
    pub worker_metrics: Arc<Mutex<WorkerMetrics>>,
}

impl Metrics {
    pub fn new(worker_metrics: Arc<Mutex<WorkerMetrics>>) -> Self {
        let uptime_seconds = MetricTypeGauge::new(
            "rmn_uptime_seconds",
            "time in seconds since the node started or internally restarted due to RMN contract onchain config changes",
        );
        let disabled_chains = MetricTypeGauge::new(
            "rmn_disabled_chains",
            "number of chains that RMN should but does not operate with",
        );
        let sent_bless_txs =
            MetricTypeCounter::new("rmn_sent_bless_txs", "number of bless tx sent");
        let gas_fee_metrics = GasFeeMetrics::new();
        let chain_status_metrics = ChainStatusMetrics::new();
        let lane_status_metrics = LaneStatusMetrics::new();
        Self {
            uptime_seconds,
            disabled_chains,
            sent_bless_txs,
            gas_fee_metrics,
            chain_status_metrics,
            lane_status_metrics,
            worker_metrics,
        }
    }

    fn get_prometheus_file_lines(&self) -> Vec<String> {
        join_prometheus_grouped_lines_with_spacing(vec![
            self.uptime_seconds.get_prometheus_file_lines(),
            self.disabled_chains.get_prometheus_file_lines(),
            self.sent_bless_txs.get_prometheus_file_lines(),
            self.gas_fee_metrics.get_prometheus_file_lines(),
            self.chain_status_metrics.get_prometheus_file_lines(),
            self.lane_status_metrics.get_prometheus_file_lines(),
            self.worker_metrics
                .lock()
                .unwrap()
                .get_prometheus_file_lines(),
        ])
    }
}

fn ensure_writable_file(path: impl AsRef<Path>) -> Result<()> {
    if let Some(parent) = path.as_ref().parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create parent dirs: {:?}", parent))?;
    }
    if path.as_ref().is_dir() {
        bail!("metrics file path {:?} is a directory", path.as_ref())
    }
    File::options()
        .append(true)
        .create(true)
        .open(&path)
        .map(|_| ())
        .with_context(|| format!("file {:?} is not writable", path.as_ref()))
}

pub struct MetricsFileWorker {}
impl MetricsFileWorker {
    pub fn spawn(
        ctx: Arc<worker::Context>,
        metrics: Metrics,
        metrics_file_path: Option<PathBuf>,
    ) -> Result<(Self, ShutdownHandle)> {
        let worker_name = "MetricsFileWorker";
        let (metrics_file_path, metrics_temp_file_path) = {
            let metrics_file_path = match metrics_file_path {
                Some(file_path) => file_path,
                None => PathBuf::from_str(DEFAULT_METRICS_FILE_PATH)?,
            };
            ensure_writable_file(&metrics_file_path)?;
            if let Some(metrics_file_name) = metrics_file_path.file_name() {
                let temp_metrics_file_path = metrics_file_path
                    .with_file_name(format!("{}.temp", metrics_file_name.to_string_lossy()));
                ensure_writable_file(&temp_metrics_file_path)?;
                (metrics_file_path, temp_metrics_file_path)
            } else {
                bail!("metrics path {:?} is not a file path", metrics_file_path);
            }
        };

        let handle = {
            ctx.spawn_repeat(
                worker_name,
                METRICS_FILE_WORKER_POLL_INTERVAL,
                move |_ctx: &worker::Context| -> Result<()> {
                    let mut temp_file = File::create(&metrics_temp_file_path)?;
                    let file_lines = metrics.get_prometheus_file_lines();
                    debug!(
                        "{worker_name}: writing {} lines to metric file {}",
                        file_lines.len(),
                        metrics_file_path.display(),
                    );
                    temp_file.write_all(file_lines.join("\n").as_bytes())?;
                    temp_file.write_all(b"\n")?;
                    temp_file.flush()?;
                    fs::rename(&metrics_temp_file_path, &metrics_file_path)?;
                    Ok(())
                },
            )
        };
        Ok((Self {}, handle))
    }
}
