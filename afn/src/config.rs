use crate::{
    common::{ChainName, LaneId},
    forensics::LogRotateConfig,
};
use anyhow::{anyhow, Context, Result};
use minieth::{bytes::Address, tx_sender::TransactionSenderFeeConfig};
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    env, fs,
    hash::Hash,
    time::Duration,
};

pub const CESM_POLL_INTERVAL: Duration = Duration::from_secs(1);

pub const LANE_BLESS_STATUS_POLL_INTERVAL: Duration = Duration::from_secs(1);

pub const CHAIN_STATUS_WORKER_POLL_INTERVAL: Duration = Duration::from_secs(1);

pub const CHAIN_STATUS_WORKER_UPDATE_MAX_CONSECUTIVE_FAILURES: usize = 10;

pub const ONCHAIN_CONFIG_DISCOVERY_WORKER_POLL_INTERVAL: Duration = Duration::from_secs(30);

pub const OFFRAMP_ANOMALY_DETECTOR_POLL_INTERVAL: Duration = Duration::from_secs(1);

pub const VOTE_TO_BLESS_WORKER_POLL_INTERVAL: Duration = Duration::from_secs(5);

pub const VOTE_TO_CURSE_WORKER_POLL_INTERVAL: Duration = Duration::from_secs(1);

pub const REAPER_POLL_INTERVAL: Duration = Duration::from_secs(30);

pub const STATE_MONITOR_INTERVAL: Duration = Duration::from_secs(1);

pub const MIN_STATUS_REPORT_INTERVAL: Duration = Duration::from_secs(10);

pub const CONFIG_DISCOVERY_RETRY_INTERVAL: Duration = Duration::from_secs(5);

pub const METRICS_FILE_WORKER_POLL_INTERVAL: Duration = Duration::from_secs(1);

pub const GAS_FEE_METRICS_UPDATE_WORKER_POLL_INTERVAL: Duration = Duration::from_secs(10);

pub const VOTE_TO_CURSE_SEND_INTERVAL_PER_CURSE_ID: Duration = Duration::from_secs(30);

pub const FORENSICS_LOGROTATE_CONFIG: LogRotateConfig = LogRotateConfig {
    root_dir: Cow::Borrowed("./forensics"),
    rotatable_log_file_age: Duration::from_secs(60 * 60),
    rotatable_uncompressed_log_file_size: 5_000_000_000,
    reapable_log_file_age: Duration::from_secs(60 * 60 * 24 * 2),
    failed_rotate_cooldown: Duration::from_secs(60),
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum ChainStability {
    ConfirmationDepth {
        soft_confirmations: u64,
        hard_confirmations: u64,
    },
    FinalityTag {
        soft_confirmations: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct SharedChainConfig {
    pub name: ChainName,
    pub stability: ChainStability,
    pub bless_fee_config: TransactionSenderFeeConfig,
    pub curse_fee_config: TransactionSenderFeeConfig,
    pub max_tagged_roots_per_vote_to_bless: usize,
    pub afn_contract: Address,
    pub inflight_time: TimeUnit,
    pub max_fresh_block_age: TimeUnit,
    #[serde(default)]
    pub upon_finality_violation_vote_to_curse_on_other_chains: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum LaneType {
    Evm2EvmV1_0,
    Evm2EvmV1_2,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SharedLaneConfig {
    #[serde(flatten)]
    pub lane_id: LaneId,
    #[serde(alias = "type")]
    pub lane_type: LaneType,
    pub source_start_block_number: u64,
    pub dest_start_block_number: u64,
    pub commit_store: Address,
    pub on_ramp: Address,
    pub off_ramp: Address,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct QualifiedAddress {
    pub chain: ChainName,
    pub address: Address,
}

impl From<(ChainName, Address)> for QualifiedAddress {
    fn from(chain_and_address: (ChainName, Address)) -> QualifiedAddress {
        QualifiedAddress {
            chain: chain_and_address.0,
            address: chain_and_address.1,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalChainConfig {
    pub name: ChainName,
    #[serde(default)]
    pub disabled: bool,
    pub rpcs: Vec<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum TimeUnit {
    Seconds(u64),
    Minutes(u64),
}

impl From<TimeUnit> for Duration {
    fn from(value: TimeUnit) -> Self {
        match value {
            TimeUnit::Seconds(seconds) => Duration::from_secs(seconds),
            TimeUnit::Minutes(minutes) => Duration::from_secs(minutes * 60),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct LocalOffchainConfig {
    chains: Vec<LocalChainConfig>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct SharedOffchainConfig {
    chains: Vec<SharedChainConfig>,
    lanes: Vec<SharedLaneConfig>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OffchainConfig {
    pub shared_config_by_enabled_chain: HashMap<ChainName, SharedChainConfig>,
    pub shared_config_by_disabled_chain: HashMap<ChainName, SharedChainConfig>,
    pub shared_config_by_enabled_lane: HashMap<LaneId, SharedLaneConfig>,
    pub local_config_by_enabled_chain: HashMap<ChainName, LocalChainConfig>,
}

impl OffchainConfig {
    pub fn new(
        shared_chain_configs: Vec<SharedChainConfig>,
        shared_lane_configs: Vec<SharedLaneConfig>,
        local_chain_configs: Vec<LocalChainConfig>,
    ) -> Result<OffchainConfig> {
        if let Some((_, name)) = find_duplicate(&local_chain_configs, |c| c.name) {
            return Err(anyhow!("duplicate local config entry for chain {:?}", name));
        }

        let local_config_by_chain: HashMap<_, _> = local_chain_configs
            .into_iter()
            .map(|c| (c.name, c))
            .collect();

        if let Some((_, name)) = find_duplicate(&shared_chain_configs, |c| c.name) {
            return Err(anyhow!(
                "duplicate shared config entry for chain {:?}",
                name
            ));
        }

        let shared_config_by_chain: HashMap<_, _> = shared_chain_configs
            .into_iter()
            .map(|c| (c.name, c))
            .collect();

        for chain in local_config_by_chain.keys() {
            if !shared_config_by_chain.contains_key(chain) {
                return Err(anyhow!("local config entry for chain {chain:?} has no corresponding shared config entry"));
            }
        }
        for chain in shared_config_by_chain.keys() {
            if !local_config_by_chain.contains_key(chain) {
                return Err(anyhow!("shared config entry for chain {chain:?} has no corresponding local config entry"));
            }
        }

        if let Some((_, lane_id)) = find_duplicate(&shared_lane_configs, |c| c.lane_id.clone()) {
            return Err(anyhow!("duplicate entry for lane {:?}", lane_id));
        }

        if let Some((_, qualified_commit_store)) = find_duplicate(&shared_lane_configs, |c| {
            QualifiedAddress::from((c.lane_id.dest_chain_name, c.commit_store))
        }) {
            return Err(anyhow!(
                "duplicate entry for commit store {:?}",
                qualified_commit_store
            ));
        }

        for lane_config @ SharedLaneConfig { lane_id, .. } in &shared_lane_configs {
            for chain in [lane_id.source_chain_name, lane_id.dest_chain_name] {
                if !shared_config_by_chain.contains_key(&chain) {
                    return Err(anyhow!(
                        "lane config {:?} refers to unknown chain {:?}",
                        lane_config,
                        chain,
                    ));
                }
            }

            if lane_id.source_chain_name == lane_id.dest_chain_name {
                return Err(anyhow!(
                    "lane config {:?} has matching source and dest",
                    lane_config
                ));
            }
        }

        let local_config_by_enabled_chain: HashMap<_, _> = local_config_by_chain
            .into_iter()
            .filter(|(_, c)| !c.disabled)
            .collect();

        let (shared_config_by_enabled_chain, shared_config_by_disabled_chain): (
            HashMap<_, _>,
            HashMap<_, _>,
        ) = shared_config_by_chain
            .into_iter()
            .partition(|(chain, _)| local_config_by_enabled_chain.contains_key(chain));

        let shared_config_by_enabled_lane: HashMap<LaneId, SharedLaneConfig> = shared_lane_configs
            .into_iter()
            .filter(|c| {
                shared_config_by_enabled_chain.contains_key(&c.lane_id.source_chain_name)
                    && shared_config_by_enabled_chain.contains_key(&c.lane_id.dest_chain_name)
            })
            .map(|c| (c.lane_id.clone(), c))
            .collect();

        Ok(OffchainConfig {
            shared_config_by_enabled_chain,
            shared_config_by_disabled_chain,
            shared_config_by_enabled_lane,
            local_config_by_enabled_chain,
        })
    }

    pub fn dest_chains(&self) -> HashMap<ChainName, &SharedChainConfig> {
        let mut h = HashMap::new();
        for (lane_id, _) in self.shared_config_by_enabled_lane.iter() {
            h.insert(
                lane_id.dest_chain_name,
                self.shared_config_by_enabled_chain
                    .get(&lane_id.dest_chain_name)
                    .unwrap(),
            );
        }
        h
    }

    pub fn enabled_chains(&self) -> Vec<ChainName> {
        self.shared_config_by_enabled_chain
            .keys()
            .copied()
            .collect()
    }

    pub fn enabled_lanes(&self) -> Vec<LaneId> {
        self.shared_config_by_enabled_lane.keys().cloned().collect()
    }

    pub fn disabled_chains(&self) -> Vec<ChainName> {
        self.shared_config_by_disabled_chain
            .keys()
            .copied()
            .collect()
    }
}

fn find_duplicate<T, K: Eq + Hash, F>(list: &[T], key_fn: F) -> Option<(&T, K)>
where
    F: Fn(&T) -> K,
{
    let mut key_set = HashSet::<K>::new();
    for item in list {
        let key = key_fn(item);
        if key_set.contains(&key) {
            return Some((item, key));
        }
        key_set.insert(key);
    }
    None
}

const SHARED_CONFIG_ENV_VAR: &str = "AFN_SHARED_CONFIG";
const LOCAL_CONFIG_ENV_VAR: &str = "AFN_LOCAL_CONFIG";

fn config_path(env_var: &str) -> Result<String> {
    env::var(env_var)
        .with_context(|| anyhow!("need to set {env_var} env variable to your toml config"))
}

pub fn load_config_files() -> Result<OffchainConfig> {
    let shared_config: SharedOffchainConfig =
        toml::from_str(&fs::read_to_string(config_path(SHARED_CONFIG_ENV_VAR)?)?)?;
    let local_config: LocalOffchainConfig =
        toml::from_str(&fs::read_to_string(config_path(LOCAL_CONFIG_ENV_VAR)?)?)?;
    OffchainConfig::new(
        shared_config.chains,
        shared_config.lanes,
        local_config.chains,
    )
}
