use crate::common::{ChainName, LaneId};
use anyhow::{anyhow, bail, Context, Result};
use minieth::{bytes::Address, tx_sender::TransactionSenderFeeConfig};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    env, fs,
    hash::Hash,
    str::FromStr,
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

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Deployment {
    Prod,
    Beta,
}

impl FromStr for Deployment {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "prod" => Ok(Self::Prod),
            "beta" => Ok(Self::Beta),
            _ => Err(format!("unknown deployment {:?}", s)),
        }
    }
}

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainConfig {
    pub name: ChainName,
    pub stability: ChainStability,
    pub bless_fee_config: TransactionSenderFeeConfig,
    pub curse_fee_config: TransactionSenderFeeConfig,
    pub max_tagged_roots_per_vote_to_bless: usize,
    pub afn_contract: Address,
    pub inflight_time: TimeUnit,
    pub max_fresh_block_age: TimeUnit,
    pub source_contracts_getlogs_max_block_range: u64,
    pub dest_contracts_getlogs_max_block_range: u64,
    pub rpcs: Vec<String>,
}

impl ChainConfig {
    fn new(
        shared_chain_config: &SharedChainConfig,
        local_chain_config: &LocalChainConfig,
    ) -> Result<Self> {
        if shared_chain_config.name != local_chain_config.name {
            bail!(
                "inconsistent chain names from shared and local chain config: shared={} local={}",
                shared_chain_config.name,
                local_chain_config.name
            )
        }
        Ok(Self {
            name: shared_chain_config.name,
            stability: shared_chain_config.stability,
            bless_fee_config: shared_chain_config.bless_fee_config,
            curse_fee_config: shared_chain_config.curse_fee_config,
            max_tagged_roots_per_vote_to_bless: shared_chain_config
                .max_tagged_roots_per_vote_to_bless,
            afn_contract: shared_chain_config.afn_contract,
            inflight_time: shared_chain_config.inflight_time,
            max_fresh_block_age: shared_chain_config.max_fresh_block_age,
            source_contracts_getlogs_max_block_range: shared_chain_config
                .source_contracts_getlogs_max_block_range,
            dest_contracts_getlogs_max_block_range: shared_chain_config
                .dest_contracts_getlogs_max_block_range,
            rpcs: local_chain_config.rpcs.clone(),
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LaneConfig {
    #[serde(flatten)]
    pub lane_id: LaneId,
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
struct LocalChainConfig {
    pub name: ChainName,
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct SharedChainConfig {
    pub name: ChainName,
    pub stability: ChainStability,
    pub bless_fee_config: TransactionSenderFeeConfig,
    pub curse_fee_config: TransactionSenderFeeConfig,
    pub max_tagged_roots_per_vote_to_bless: usize,
    pub afn_contract: Address,
    pub inflight_time: TimeUnit,
    pub max_fresh_block_age: TimeUnit,
    pub source_contracts_getlogs_max_block_range: u64,
    pub dest_contracts_getlogs_max_block_range: u64,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct LocalOffchainConfig {
    chains: Vec<LocalChainConfig>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct SharedOffchainConfig {
    chains: Vec<SharedChainConfig>,
    lanes: Vec<LaneConfig>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OffchainConfig {
    pub chains: HashMap<ChainName, ChainConfig>,
    pub lanes: HashMap<LaneId, LaneConfig>,
}

impl OffchainConfig {
    fn new_from_shared_and_local(
        shared_config: SharedOffchainConfig,
        local_config: LocalOffchainConfig,
    ) -> Result<Self> {
        if let Some((_, name)) = find_duplicate(&shared_config.chains, |c| c.name) {
            return Err(anyhow!(
                "shared_config: duplicate entry for chain {:?}",
                name
            ));
        }
        if let Some((_, name)) = find_duplicate(&local_config.chains, |c| c.name) {
            return Err(anyhow!(
                "local_config: duplicate entry for chain {:?}",
                name
            ));
        }
        let local_chain_configs: HashMap<ChainName, LocalChainConfig> = HashMap::from_iter(
            local_config
                .chains
                .into_iter()
                .map(|config| (config.name, config)),
        );
        let mut chain_configs = Vec::with_capacity(shared_config.chains.len());
        for shared_chain_config in &shared_config.chains {
            let local_chain_config =
                local_chain_configs
                    .get(&shared_chain_config.name)
                    .ok_or(anyhow!(
                        "local chain config not found: {}",
                        shared_chain_config.name
                    ))?;
            chain_configs.push(ChainConfig::new(shared_chain_config, local_chain_config)?)
        }
        Self::new(chain_configs, shared_config.lanes)
    }

    pub fn new(
        chain_configs: Vec<ChainConfig>,
        lane_configs: Vec<LaneConfig>,
    ) -> Result<OffchainConfig> {
        if let Some((_, name)) = find_duplicate(&chain_configs, |c| c.name) {
            return Err(anyhow!("duplicate entry for chain {:?}", name));
        }

        let chains: HashMap<ChainName, ChainConfig> =
            chain_configs.into_iter().map(|c| (c.name, c)).collect();

        if let Some((_, lane_id)) = find_duplicate(&lane_configs, |c| c.lane_id.clone()) {
            return Err(anyhow!("duplicate entry for lane {:?}", lane_id));
        }

        if let Some((_, qualified_commit_store)) = find_duplicate(&lane_configs, |c| {
            QualifiedAddress::from((c.lane_id.dest_chain_name, c.commit_store))
        }) {
            return Err(anyhow!(
                "duplicate entry for commit store {:?}",
                qualified_commit_store
            ));
        }

        if let Some(lane_config) = lane_configs.iter().find(|c| {
            !(chains.contains_key(&c.lane_id.source_chain_name)
                && chains.contains_key(&c.lane_id.dest_chain_name))
        }) {
            return Err(anyhow!(
                "lane config {:?} refers to unknown chain",
                lane_config
            ));
        }

        if let Some(lane_config) = lane_configs
            .iter()
            .find(|c| c.lane_id.source_chain_name == c.lane_id.dest_chain_name)
        {
            return Err(anyhow!(
                "lane config {:?} has matching source and dest",
                lane_config
            ));
        }

        let lanes: HashMap<LaneId, LaneConfig> = lane_configs
            .into_iter()
            .map(|c| (c.lane_id.clone(), c))
            .collect();

        Ok(OffchainConfig { chains, lanes })
    }

    pub fn dest_chains(&self) -> HashMap<ChainName, &ChainConfig> {
        let mut h = HashMap::new();
        for (lane_id, _) in self.lanes.iter() {
            h.insert(
                lane_id.dest_chain_name,
                self.chains.get(&lane_id.dest_chain_name).unwrap(),
            );
        }
        h
    }

    pub fn all_chains(&self) -> Vec<ChainName> {
        self.chains.keys().copied().collect()
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
    OffchainConfig::new_from_shared_and_local(shared_config, local_config)
}
