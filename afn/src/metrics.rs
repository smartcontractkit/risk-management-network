use anyhow::{bail, Context, Result};
use core::hash::Hash;
use std::{
    collections::HashMap,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    str::FromStr,
    sync::{Arc, Mutex},
};
use tracing::debug;

const DEFAULT_METRICS_FILE_PATH: &str = "./metrics/rmn-metrics.prom";

use crate::{
    common::ChainName,
    config::METRICS_WORKER_POLL_INTERVAL,
    worker::{self, ShutdownHandle},
};

#[derive(Debug, Eq, Hash, PartialEq, Clone)]
struct MetricLabel {
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
pub struct MetricSample {
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

trait LabelKey: Hash + Eq {
    fn to_metric_labels(&self) -> Vec<MetricLabel>;
}

impl LabelKey for ChainName {
    fn to_metric_labels(&self) -> Vec<MetricLabel> {
        vec![MetricLabel::new("chain_name", &self.to_string())]
    }
}

trait Metric<K: LabelKey> {
    fn new(name: &str, doc_string: &str, label_keys: Vec<K>) -> Self;
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

struct MetricTypeCounter<K: LabelKey> {
    name: String,
    doc_string: String,
    counter_value_by_key: HashMap<K, u64>,
}

impl<K: LabelKey> Metric<K> for MetricTypeCounter<K> {
    fn new(name: &str, doc_string: &str, label_keys: Vec<K>) -> Self {
        Self {
            name: name.to_string(),
            doc_string: doc_string.to_string(),
            counter_value_by_key: HashMap::from_iter(label_keys.into_iter().map(|k| (k, 0u64))),
        }
    }
    fn help_and_type_lines(&self) -> Vec<String> {
        get_help_and_type_lines_helper(&self.name, MetricType::Counter, &self.doc_string)
    }
    fn take_samples(&self) -> Vec<MetricSample> {
        self.counter_value_by_key
            .iter()
            .map(|(k, v)| MetricSample {
                name: self.name.clone(),
                labels: k.to_metric_labels(),
                value: *v as f64,
            })
            .collect()
    }
}

impl<K: LabelKey> MetricTypeCounter<K> {
    fn increase_by(&mut self, label_key: K, inc_value: u64) {
        match self.counter_value_by_key.get_mut(&label_key) {
            None => {}
            Some(value) => *value += inc_value,
        }
    }
}

struct MetricTypeGauge<K: LabelKey> {
    name: String,
    doc_string: String,
    gauge_value_by_key: HashMap<K, Option<f64>>,
}

impl<K: LabelKey> Metric<K> for MetricTypeGauge<K> {
    fn new(name: &str, doc_string: &str, label_keys: Vec<K>) -> Self {
        Self {
            name: name.to_string(),
            doc_string: doc_string.to_string(),
            gauge_value_by_key: HashMap::from_iter(label_keys.into_iter().map(|k| (k, None))),
        }
    }
    fn help_and_type_lines(&self) -> Vec<String> {
        get_help_and_type_lines_helper(&self.name, MetricType::Gauge, &self.doc_string)
    }
    fn take_samples(&self) -> Vec<MetricSample> {
        self.gauge_value_by_key
            .iter()
            .filter_map(|(k, v)| {
                v.map(|v| MetricSample {
                    name: self.name.clone(),
                    labels: k.to_metric_labels(),
                    value: v,
                })
            })
            .collect()
    }
}

impl<K: LabelKey> MetricTypeGauge<K> {
    fn set_value(&mut self, label_key: K, new_value: f64) {
        match self.gauge_value_by_key.get_mut(&label_key) {
            None => {}
            Some(value) => *value = Some(new_value),
        }
    }
}

struct Metrics {
    pub bless_transaction_counter: MetricTypeCounter<ChainName>,
    pub latest_block_number_gauge: MetricTypeGauge<ChainName>,
    pub finalized_block_number_gauge: MetricTypeGauge<ChainName>,
}

impl Metrics {
    fn new(chains: Vec<ChainName>) -> Self {
        let bless_transaction_counter = MetricTypeCounter::new(
            "rmn_bless_tx_count",
            "number of bless tx sent",
            chains.clone(),
        );
        let latest_block_number_gauge = MetricTypeGauge::new(
            "rmn_latest_block_number",
            "block number of latest block",
            chains.clone(),
        );
        let finalized_block_number_gauge = MetricTypeGauge::new(
            "rmn_finalized_block_number",
            "block number of finalized block",
            chains,
        );
        Self {
            bless_transaction_counter,
            latest_block_number_gauge,
            finalized_block_number_gauge,
        }
    }

    fn get_prometheus_file_lines(&mut self) -> Vec<String> {
        [
            self.bless_transaction_counter.get_prometheus_file_lines(),
            self.latest_block_number_gauge.get_prometheus_file_lines(),
            self.finalized_block_number_gauge
                .get_prometheus_file_lines(),
        ]
        .into_iter()
        .flat_map(|v| [v, vec![String::new()]])
        .flatten()
        .collect()
    }
}

pub trait ChainMetrics {
    fn inc_bless_tx_counter(&self);
    fn set_latest_block_number(&self, latest_block_number: u64);
    fn set_finalized_block_number(&self, finalized_block_number: u64);
}

pub struct ChainMetricsHandle {
    metrics: Arc<Mutex<Metrics>>,
    chain_name: ChainName,
}
impl ChainMetrics for ChainMetricsHandle {
    fn inc_bless_tx_counter(&self) {
        self.metrics
            .lock()
            .unwrap()
            .bless_transaction_counter
            .increase_by(self.chain_name, 1)
    }
    fn set_latest_block_number(&self, latest_block_number: u64) {
        self.metrics
            .lock()
            .unwrap()
            .latest_block_number_gauge
            .set_value(self.chain_name, latest_block_number as f64)
    }
    fn set_finalized_block_number(&self, finalized_block_number: u64) {
        self.metrics
            .lock()
            .unwrap()
            .finalized_block_number_gauge
            .set_value(self.chain_name, finalized_block_number as f64)
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

pub struct MetricsWorker {
    metrics: Arc<Mutex<Metrics>>,
}
impl MetricsWorker {
    pub fn spawn(
        ctx: Arc<worker::Context>,
        chains: Vec<ChainName>,
        metrics_file_path: Option<PathBuf>,
    ) -> Result<(Self, ShutdownHandle)> {
        let metrics = Arc::new(Mutex::new(Metrics::new(chains)));
        let worker_name = "MetricsWorker";
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
            let metrics = Arc::clone(&metrics);
            ctx.spawn_repeat(
                worker_name,
                METRICS_WORKER_POLL_INTERVAL,
                move |_ctx: &worker::Context| -> Result<()> {
                    let mut temp_file = File::create(&metrics_temp_file_path)?;
                    let file_lines = metrics.lock().unwrap().get_prometheus_file_lines();
                    debug!(
                        "{worker_name}: writing {} lines to metric file {}",
                        file_lines.len(),
                        metrics_file_path.display(),
                    );
                    temp_file.write_all(file_lines.join("\n").as_bytes())?;
                    temp_file.flush()?;
                    fs::rename(&metrics_temp_file_path, &metrics_file_path)?;
                    Ok(())
                },
            )
        };
        Ok((Self { metrics }, handle))
    }

    pub fn make_chain_metrics_handle(&self, chain_name: ChainName) -> ChainMetricsHandle {
        ChainMetricsHandle {
            metrics: Arc::clone(&self.metrics),
            chain_name,
        }
    }
}
