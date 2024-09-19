use miniabi::{abi_decode::AbiDecode, types::Value};
use minieth::{bytes::Address, rpc::Rpc};

use crate::{
    chain_selector::ChainSelector,
    commit_store,
    common::{ContractCall, EncodeContractCall},
    config::LaneType,
    evm2evm_offramp_v1_2, evm2evm_offramp_v1_5, evm2evm_onramp_v1_2, evm2evm_onramp_v1_5,
};

#[derive(Debug, PartialEq, Eq)]
pub struct OnRampStaticConfig {
    pub chain_selector: ChainSelector,
    pub dest_chain_selector: ChainSelector,
}

type OnRarmpStaticConfigV1_2 = evm2evm_onramp_v1_2::StaticConfigV1_2;

impl From<OnRarmpStaticConfigV1_2> for OnRampStaticConfig {
    fn from(value: OnRarmpStaticConfigV1_2) -> Self {
        Self {
            chain_selector: value.chain_selector,
            dest_chain_selector: value.dest_chain_selector,
        }
    }
}

type OnRampStaticConfigV1_5 = evm2evm_onramp_v1_5::StaticConfigV1_5;

impl From<OnRampStaticConfigV1_5> for OnRampStaticConfig {
    fn from(value: OnRampStaticConfigV1_5) -> Self {
        Self {
            chain_selector: value.chain_selector,
            dest_chain_selector: value.dest_chain_selector,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct OffRampStaticConfig {
    pub chain_selector: ChainSelector,
    pub src_chain_selector: ChainSelector,
    pub commit_store: Address,
    pub on_ramp: Address,
}

type OffRampStaticConfigV1_2 = evm2evm_offramp_v1_2::StaticConfigV1_2;

impl From<OffRampStaticConfigV1_2> for OffRampStaticConfig {
    fn from(value: OffRampStaticConfigV1_2) -> Self {
        Self {
            chain_selector: value.chain_selector,
            src_chain_selector: value.src_chain_selector,
            commit_store: value.commit_store,
            on_ramp: value.on_ramp,
        }
    }
}

type OffRampStaticConfigV1_5 = evm2evm_offramp_v1_5::StaticConfigV1_5;

impl From<OffRampStaticConfigV1_5> for OffRampStaticConfig {
    fn from(value: OffRampStaticConfigV1_5) -> Self {
        Self {
            chain_selector: value.chain_selector,
            src_chain_selector: value.src_chain_selector,
            commit_store: value.commit_store,
            on_ramp: value.on_ramp,
        }
    }
}

pub type CommitStoreStaticConfig = commit_store::StaticConfig;

struct GetStaticConfig();

impl ContractCall for GetStaticConfig {
    fn contract_call_signature() -> &'static str {
        "getStaticConfig()"
    }

    fn contract_call_parameters(self) -> Value {
        ().into()
    }
}

fn fetch_onchain_static_config<T: AbiDecode>(rpc: &Rpc, contract: Address) -> anyhow::Result<T> {
    Ok(T::abi_decode(rpc.call(
        contract,
        GetStaticConfig {}.encode_contract_call(),
    )?)?)
}

pub fn onchain_onramp_static_config(
    rpc: &Rpc,
    on_ramp: Address,
    lane_type: LaneType,
) -> anyhow::Result<OnRampStaticConfig> {
    match lane_type {
        LaneType::Evm2EvmV1_2 => {
            fetch_onchain_static_config::<OnRarmpStaticConfigV1_2>(rpc, on_ramp)
                .map(OnRampStaticConfig::from)
        }
        LaneType::Evm2EvmV1_5 => {
            fetch_onchain_static_config::<OnRampStaticConfigV1_5>(rpc, on_ramp)
                .map(OnRampStaticConfig::from)
        }
    }
}

pub fn onchain_offramp_static_config(
    rpc: &Rpc,
    off_ramp: Address,
    lane_type: LaneType,
) -> anyhow::Result<OffRampStaticConfig> {
    match lane_type {
        LaneType::Evm2EvmV1_2 => {
            fetch_onchain_static_config::<OffRampStaticConfigV1_2>(rpc, off_ramp)
                .map(OffRampStaticConfig::from)
        }
        LaneType::Evm2EvmV1_5 => {
            fetch_onchain_static_config::<OffRampStaticConfigV1_5>(rpc, off_ramp)
                .map(OffRampStaticConfig::from)
        }
    }
}

pub fn onchain_commit_store_static_config(
    rpc: &Rpc,
    commit_store: Address,
) -> anyhow::Result<CommitStoreStaticConfig> {
    fetch_onchain_static_config(rpc, commit_store)
}

struct TypeAndVersion();

impl ContractCall for TypeAndVersion {
    fn contract_call_signature() -> &'static str {
        "typeAndVersion()"
    }

    fn contract_call_parameters(self) -> Value {
        ().into()
    }
}

pub fn onchain_type_and_version(rpc: &Rpc, contract: Address) -> anyhow::Result<String> {
    Ok(String::abi_decode(rpc.call(
        contract,
        TypeAndVersion {}.encode_contract_call(),
    )?)?)
}
