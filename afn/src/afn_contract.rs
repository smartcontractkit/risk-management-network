use crate::common::{ContractCall, EncodeContractCall, LogSignature, UncheckedDecodeLog};
use crate::config::SharedChainConfig;
use crate::curse_beacon::CurseType;
use crate::{
    afn_contract_v1_0::{get_vote_to_curse_call_and_gas_limit_v1_0, OnchainConfigV1_0, VoterV1_0},
    afn_contract_v1_5::{get_vote_to_curse_call_and_gas_limit_v1_5, OnchainConfigV1_5, VoterV1_5},
};
use anyhow::{anyhow, bail, Result};
use miniabi::{
    abi_decode::{AbiDecode, AbiDecodeError, AbiTypeToDecode},
    abi_encode::AbiEncode,
    types::{Type, Value},
};
use minieth::{
    bytes::{Address, Bytes, Bytes16, Bytes32},
    keccak::keccak256,
    rpc::{EVMLog, Rpc},
    tx_sender::{TransactionRequest, TransactionSender},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Voter {
    pub bless_vote_addr: Address,
    pub curse_vote_addr: Address,
    pub curse_unvote_addr: Option<Address>,
    pub bless_weight: u8,
    pub curse_weight: u8,
}
impl From<VoterV1_0> for Voter {
    fn from(v: VoterV1_0) -> Self {
        Voter {
            bless_vote_addr: v.bless_vote_addr,
            curse_vote_addr: v.curse_vote_addr,
            curse_unvote_addr: Some(v.curse_unvote_addr),
            bless_weight: v.bless_weight,
            curse_weight: v.curse_weight,
        }
    }
}
impl From<VoterV1_5> for Voter {
    fn from(v: VoterV1_5) -> Self {
        Voter {
            bless_vote_addr: v.bless_vote_addr,
            curse_vote_addr: v.curse_vote_addr,
            curse_unvote_addr: None,
            bless_weight: v.bless_weight,
            curse_weight: v.curse_weight,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct OnchainConfig {
    pub voters: Vec<Voter>,
    pub bless_weight_threshold: u16,
    pub curse_weight_threshold: u16,
}

impl From<OnchainConfigV1_0> for OnchainConfig {
    fn from(v: OnchainConfigV1_0) -> Self {
        OnchainConfig {
            voters: v.voters.into_iter().map(|v| v.into()).collect(),
            bless_weight_threshold: v.bless_weight_threshold,
            curse_weight_threshold: v.curse_weight_threshold,
        }
    }
}
impl From<OnchainConfigV1_5> for OnchainConfig {
    fn from(v: OnchainConfigV1_5) -> Self {
        OnchainConfig {
            voters: v.voters.into_iter().map(|v| v.into()).collect(),
            bless_weight_threshold: v.bless_weight_threshold,
            curse_weight_threshold: v.curse_weight_threshold,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaggedRootBlessedEvent {
    pub config_version: u32,
    pub tagged_root: TaggedRoot,
    pub accumulated_weight: u16,
}
impl LogSignature for TaggedRootBlessedEvent {
    fn log_signature() -> &'static str {
        "TaggedRootBlessed(uint32,(address,bytes32),uint16)"
    }
}

impl UncheckedDecodeLog for TaggedRootBlessedEvent {
    fn unchecked_decode_log(log: EVMLog) -> Result<Self> {
        let config_version = u32::abi_decode(log.topics.get(1).ok_or_else(|| {
            anyhow!(
                "{}: log.topics[1] is needed to parse [config_version]",
                Self::log_signature()
            )
        })?)?;
        let (tagged_root, accumulated_weight) = <(TaggedRoot, u16)>::abi_decode(log.data)?;
        Ok(Self {
            config_version,
            tagged_root,
            accumulated_weight,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaggedRootBlessVotesResetEvent {
    pub config_version: u32,
    pub tagged_root: TaggedRoot,
    pub was_blessed: bool,
}

impl LogSignature for TaggedRootBlessVotesResetEvent {
    fn log_signature() -> &'static str {
        "TaggedRootBlessVotesReset(uint32,(address,bytes32),bool)"
    }
}

impl UncheckedDecodeLog for TaggedRootBlessVotesResetEvent {
    fn unchecked_decode_log(log: EVMLog) -> Result<Self> {
        let config_version = u32::abi_decode(log.topics.get(1).ok_or_else(|| {
            anyhow!(
                "{}: log.topics[1] is needed to parse [config_version]",
                Self::log_signature()
            )
        })?)?;
        let (tagged_root, was_blessed) = <(TaggedRoot, bool)>::abi_decode(log.data)?;
        Ok(Self {
            config_version,
            tagged_root,
            was_blessed,
        })
    }
}

fn parse_indexed_config_version_voter_and_root_as_tuple(
    log_signature: &str,
    topics: &[Bytes32],
) -> Result<(u32, Address)> {
    let config_version = u32::abi_decode(topics.get(1).ok_or_else(|| {
        anyhow!(
            "{}: log.topics[1] is needed to parse [config_version]",
            log_signature
        )
    })?)?;
    let voter = Address::abi_decode(topics.get(2).ok_or_else(|| {
        anyhow!(
            "{}: log.topics[2] is needed to parse [voter]",
            log_signature
        )
    })?)?;
    Ok((config_version, voter))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VotedToBlessEvent {
    pub config_version: u32,
    pub voter: Address,
    pub tagged_root: TaggedRoot,
    pub weight: u8,
}

impl LogSignature for VotedToBlessEvent {
    fn log_signature() -> &'static str {
        "VotedToBless(uint32,address,(address,bytes32),uint8)"
    }
}

impl UncheckedDecodeLog for VotedToBlessEvent {
    fn unchecked_decode_log(log: EVMLog) -> Result<Self> {
        let (config_version, voter) = parse_indexed_config_version_voter_and_root_as_tuple(
            Self::log_signature(),
            &log.topics,
        )?;
        let (tagged_root, weight) = <(TaggedRoot, u8)>::abi_decode(log.data)?;
        Ok(Self {
            config_version,
            voter,
            tagged_root,
            weight,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlreadyVotedToBlessEvent {
    pub config_version: u32,
    pub voter: Address,
    pub tagged_root: TaggedRoot,
}
impl LogSignature for AlreadyVotedToBlessEvent {
    fn log_signature() -> &'static str {
        "AlreadyVotedToBless(uint32,address,(address,bytes32))"
    }
}

impl UncheckedDecodeLog for AlreadyVotedToBlessEvent {
    fn unchecked_decode_log(log: EVMLog) -> Result<Self> {
        let (config_version, voter) = parse_indexed_config_version_voter_and_root_as_tuple(
            Self::log_signature(),
            &log.topics,
        )?;
        Ok(Self {
            config_version,
            voter,
            tagged_root: TaggedRoot::abi_decode(log.data)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlreadyBlessedEvent {
    pub config_version: u32,
    pub voter: Address,
    pub tagged_root: TaggedRoot,
}

impl LogSignature for AlreadyBlessedEvent {
    fn log_signature() -> &'static str {
        "AlreadyBlessed(uint32,address,(address,bytes32))"
    }
}

impl UncheckedDecodeLog for AlreadyBlessedEvent {
    fn unchecked_decode_log(log: EVMLog) -> Result<Self> {
        let (config_version, voter) = parse_indexed_config_version_voter_and_root_as_tuple(
            Self::log_signature(),
            &log.topics,
        )?;
        Ok(Self {
            config_version,
            voter,
            tagged_root: TaggedRoot::abi_decode(log.data)?,
        })
    }
}

#[derive(Debug, Eq, PartialEq, PartialOrd, Ord, Clone, Copy, Hash)]
pub struct TaggedRoot {
    pub commit_store: Address,
    pub root: Bytes32,
}
type TaggedRootAsTuple = (Address, Bytes32);

impl AbiTypeToDecode for TaggedRoot {
    fn abi_type_to_decode() -> Type {
        TaggedRootAsTuple::abi_type_to_decode()
    }
}
impl TryFrom<Value> for TaggedRoot {
    type Error = AbiDecodeError;
    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let (commit_store, root) = TaggedRootAsTuple::try_from(value)?;
        Ok(Self { commit_store, root })
    }
}

impl From<TaggedRoot> for Value {
    fn from(v: TaggedRoot) -> Self {
        (v.commit_store, v.root).into()
    }
}

impl Default for TaggedRoot {
    fn default() -> Self {
        Self {
            commit_store: [0; 20].into(),
            root: [0; 32].into(),
        }
    }
}

impl TaggedRoot {
    pub fn tagged_root_hash(self) -> Bytes32 {
        keccak256(self.abi_encode().as_ref()).into()
    }
}

pub struct VoteToBlessCall {
    pub tagged_roots: Vec<TaggedRoot>,
}
impl ContractCall for VoteToBlessCall {
    fn contract_call_signature() -> &'static str {
        "voteToBless((address,bytes32)[])"
    }
    fn contract_call_parameters(self) -> Value {
        (self.tagged_roots,).into()
    }
}

pub struct GetConfigDetailsCall;
impl ContractCall for GetConfigDetailsCall {
    fn contract_call_signature() -> &'static str {
        "getConfigDetails()"
    }
    fn contract_call_parameters(self) -> Value {
        ().into()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionedOnchainConfig {
    pub version: u32,
    pub block_number: u32,
    pub config: OnchainConfig,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AFNType {
    V1_0,
    V1_5,
}

#[derive(Clone)]
pub struct AFNInterface {
    pub address: Address,
    pub rpc: Arc<Rpc>,
    pub afn_type: AFNType,
}

impl AFNInterface {
    pub fn create_from_chain_config(rpc: Arc<Rpc>, config: &SharedChainConfig) -> Result<Self> {
        Ok(AFNInterface {
            address: config.afn_contract,
            rpc,
            afn_type: config.afn_type,
        })
    }

    fn send_transaction(
        &self,
        tx_sender: &mut TransactionSender,
        data: Bytes,
        gas: u64,
    ) -> Result<Bytes32> {
        let transaction_request = TransactionRequest {
            gas,
            to: self.address,
            value: 0u64.into(),
            data,
        };
        Ok(tx_sender.send(transaction_request)?)
    }

    pub fn get_versioned_onchain_config(&self) -> Result<VersionedOnchainConfig> {
        match self.afn_type {
            AFNType::V1_0 => {
                let data: Bytes = GetConfigDetailsCall.encode_contract_call();
                let (version, block_number, config_v1_0) = {
                    let return_data = self.rpc.call(self.address, data)?;
                    <(u32, u32, OnchainConfigV1_0)>::abi_decode(return_data)?
                };
                Ok(VersionedOnchainConfig {
                    version,
                    block_number,
                    config: config_v1_0.into(),
                })
            }
            AFNType::V1_5 => {
                let data: Bytes = GetConfigDetailsCall.encode_contract_call();
                let (version, block_number, config_v1_5) = {
                    let return_data = self.rpc.call(self.address, data)?;
                    <(u32, u32, OnchainConfigV1_5)>::abi_decode(return_data)?
                };
                Ok(VersionedOnchainConfig {
                    version,
                    block_number,
                    config: config_v1_5.into(),
                })
            }
        }
    }

    pub fn send_vote_to_curse(
        &self,
        tx_sender: &mut TransactionSender,
        curse_id: Bytes16,
        curse_type: &CurseType,
    ) -> Result<Bytes32> {
        match (self.afn_type, curse_type) {
            (AFNType::V1_0, CurseType::V1_0) => {
                let (vote_to_curse_call_v1_0, gas_limit) =
                    get_vote_to_curse_call_and_gas_limit_v1_0(curse_id);
                self.send_transaction(
                    tx_sender,
                    vote_to_curse_call_v1_0.encode_contract_call(),
                    gas_limit,
                )
            }
            (AFNType::V1_5, CurseType::V1_5(curse_subjects)) => {
                let (vote_to_curse_call_v1_5, gas_limit) =
                    get_vote_to_curse_call_and_gas_limit_v1_5(curse_id, curse_subjects)?;
                self.send_transaction(
                    tx_sender,
                    vote_to_curse_call_v1_5.encode_contract_call(),
                    gas_limit,
                )
            }
            (AFNType::V1_0, CurseType::V1_5(_)) | (AFNType::V1_5, CurseType::V1_0) => {
                bail!(
                    "unsupported curse type {:?} for RMN contract type {:?}",
                    curse_type,
                    self.afn_type
                )
            }
        }
    }
    pub fn send_vote_to_bless(
        &self,
        tx_sender: &mut TransactionSender,
        votes: Vec<TaggedRoot>,
    ) -> Result<Bytes32> {
        match self.afn_type {
            AFNType::V1_0 | AFNType::V1_5 => {
                let gas = 70_000u64
                    .checked_mul(u64::try_from(votes.len())?)
                    .ok_or(anyhow!("gas limit can't fit into u64"))?;
                let data: Bytes = VoteToBlessCall {
                    tagged_roots: votes,
                }
                .encode_contract_call();
                self.send_transaction(tx_sender, data, gas)
            }
        }
    }
}
