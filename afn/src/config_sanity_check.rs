use miniabi::{
    abi_decode::{AbiDecode, AbiDecodeError, AbiTypeToDecode},
    types::{Type, Value},
};
use minieth::{bytes::Address, rpc::Rpc};

use crate::{
    chain_selector::ChainSelector,
    common::{ContractCall, EncodeContractCall},
};

#[derive(Debug, PartialEq, Eq)]
pub struct OnRampStaticConfig {
    pub chain_selector: ChainSelector,
    pub dest_chain_selector: ChainSelector,
}

type OnRampStaticConfigAsTuple = (Address, u64, u64, u64, u128, Address, Address);

impl AbiTypeToDecode for OnRampStaticConfig {
    fn abi_type_to_decode() -> Type {
        OnRampStaticConfigAsTuple::abi_type_to_decode()
    }
}

impl TryFrom<Value> for OnRampStaticConfig {
    type Error = AbiDecodeError;
    fn try_from(v: Value) -> Result<Self, Self::Error> {
        let (
            _link_token,
            chain_selector,
            dest_chain_selector,
            _default_tx_gas_limit,
            _max_nop_fees_juels,
            _prev_onramp,
            _arm_proxy,
        ) = OnRampStaticConfigAsTuple::try_from(v)?;
        Ok(Self {
            chain_selector,
            dest_chain_selector,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct OffRampStaticConfig {
    pub chain_selector: ChainSelector,
    pub src_chain_selector: ChainSelector,
    pub commit_store: Address,
    pub on_ramp: Address,
}

type OffRampStaticConfigAsTuple = (Address, u64, u64, Address, Address, Address);

impl AbiTypeToDecode for OffRampStaticConfig {
    fn abi_type_to_decode() -> Type {
        OffRampStaticConfigAsTuple::abi_type_to_decode()
    }
}

impl TryFrom<Value> for OffRampStaticConfig {
    type Error = AbiDecodeError;
    fn try_from(v: Value) -> Result<Self, Self::Error> {
        let (commit_store, chain_selector, src_chain_selector, on_ramp, _prev_off_ramp, _arm_proxy) =
            OffRampStaticConfigAsTuple::try_from(v)?;
        Ok(Self {
            chain_selector,
            src_chain_selector,
            commit_store,
            on_ramp,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct CommitStoreStaticConfig {
    pub chain_selector: ChainSelector,
    pub src_chain_selector: ChainSelector,
    pub on_ramp: Address,
}

type CommitStoreStaticConfigAsTuple = (u64, u64, Address, Address);

impl AbiTypeToDecode for CommitStoreStaticConfig {
    fn abi_type_to_decode() -> Type {
        CommitStoreStaticConfigAsTuple::abi_type_to_decode()
    }
}

impl TryFrom<Value> for CommitStoreStaticConfig {
    type Error = AbiDecodeError;
    fn try_from(v: Value) -> Result<Self, Self::Error> {
        let (chain_selector, src_chain_selector, on_ramp, _arm_proxy) =
            CommitStoreStaticConfigAsTuple::try_from(v)?;
        Ok(Self {
            chain_selector,
            src_chain_selector,
            on_ramp,
        })
    }
}

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
) -> anyhow::Result<OnRampStaticConfig> {
    fetch_onchain_static_config(rpc, on_ramp)
}

pub fn onchain_offramp_static_config(
    rpc: &Rpc,
    off_ramp: Address,
) -> anyhow::Result<OffRampStaticConfig> {
    fetch_onchain_static_config(rpc, off_ramp)
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
