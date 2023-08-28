use crate::common::{ContractCall, EncodeContractCall, LogSignature, UncheckedDecodeLog};
use crate::config::ChainConfig;
use anyhow::{anyhow, Result};
use miniabi::abi_encode::AbiEncode;
use miniabi::{
    abi_decode::{AbiDecode, AbiDecodeError, AbiTypeToDecode, DecodedValue, TypeToDecode},
    abi_encode::{DynamicValue, StaticValue, ValueToEncode},
};
use minieth::{
    bytes::{Address, Bytes, Bytes32},
    keccak::keccak256,
    rpc::{EVMLog, Rpc},
    tx_sender::{TransactionRequest, TransactionSender},
};
use std::sync::Arc;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Voter {
    pub bless_vote_addr: Address,
    pub curse_vote_addr: Address,
    pub curse_unvote_addr: Address,
    pub bless_weight: u8,
    pub curse_weight: u8,
}
type VoterAsTuple = (Address, Address, Address, u8, u8);
impl AbiTypeToDecode for Voter {
    fn abi_type_to_decode() -> TypeToDecode {
        VoterAsTuple::abi_type_to_decode()
    }
}
impl TryFrom<DecodedValue> for Voter {
    type Error = AbiDecodeError;
    fn try_from(v: DecodedValue) -> Result<Self, Self::Error> {
        let (bless_vote_addr, curse_vote_addr, curse_unvote_addr, bless_weight, curse_weight) =
            VoterAsTuple::try_from(v)?;
        Ok(Self {
            bless_vote_addr,
            curse_vote_addr,
            curse_unvote_addr,
            bless_weight,
            curse_weight,
        })
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct OnchainConfig {
    pub voters: Vec<Voter>,
    pub bless_weight_threshold: u16,
    pub curse_weight_threshold: u16,
}

type OnchainConfigAsTuple = (Vec<Voter>, u16, u16);

impl AbiTypeToDecode for OnchainConfig {
    fn abi_type_to_decode() -> TypeToDecode {
        OnchainConfigAsTuple::abi_type_to_decode()
    }
}

impl TryFrom<DecodedValue> for OnchainConfig {
    type Error = AbiDecodeError;
    fn try_from(value: DecodedValue) -> std::result::Result<Self, Self::Error> {
        let (voters, bless_weight_threshold, curse_weight_threshold) =
            OnchainConfigAsTuple::try_from(value)?;
        Ok(Self {
            voters,
            bless_weight_threshold,
            curse_weight_threshold,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
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

#[derive(Debug, PartialEq, Eq)]
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
#[derive(Debug, PartialEq, Eq)]
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
#[derive(Debug, PartialEq, Eq)]
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
#[derive(Debug, PartialEq, Eq)]
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
    fn abi_type_to_decode() -> TypeToDecode {
        TaggedRootAsTuple::abi_type_to_decode()
    }
}
impl TryFrom<DecodedValue> for TaggedRoot {
    type Error = AbiDecodeError;
    fn try_from(value: DecodedValue) -> Result<Self, Self::Error> {
        let (commit_store, root) = TaggedRootAsTuple::try_from(value)?;
        Ok(Self { commit_store, root })
    }
}

impl From<TaggedRoot> for StaticValue {
    fn from(tagged_root: TaggedRoot) -> Self {
        StaticValue::StaticTuple(vec![
            StaticValue::Address(tagged_root.commit_store),
            StaticValue::Bytes32(tagged_root.root),
        ])
    }
}

impl From<TaggedRoot> for ValueToEncode {
    fn from(tagged_root: TaggedRoot) -> Self {
        ValueToEncode::Static(tagged_root.into())
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
        keccak256(
            ValueToEncode::Static(StaticValue::StaticTuple(vec![
                StaticValue::Address(self.commit_store),
                StaticValue::Bytes32(self.root),
            ]))
            .abi_encode()
            .as_ref(),
        )
        .into()
    }
}

struct VoteToBlessCall {
    pub tagged_roots: Vec<TaggedRoot>,
}
impl ContractCall for VoteToBlessCall {
    fn contract_call_signature() -> &'static str {
        "voteToBless((address,bytes32)[])"
    }
    fn contract_call_parameters(self) -> ValueToEncode {
        ValueToEncode::Dynamic(DynamicValue::DynamicArrayOfStaticValue(
            self.tagged_roots
                .into_iter()
                .map(|tagged_root| tagged_root.into())
                .collect(),
        ))
    }
}

struct VoteToCurseCall {
    curse_id: Bytes32,
}
impl ContractCall for VoteToCurseCall {
    fn contract_call_signature() -> &'static str {
        "voteToCurse(bytes32)"
    }
    fn contract_call_parameters(self) -> ValueToEncode {
        ValueToEncode::Static(StaticValue::StaticTuple(vec![StaticValue::Bytes32(
            self.curse_id,
        )]))
    }
}

struct GetConfigDetailsCall;
impl ContractCall for GetConfigDetailsCall {
    fn contract_call_signature() -> &'static str {
        "getConfigDetails()"
    }
    fn contract_call_parameters(self) -> ValueToEncode {
        ValueToEncode::Static(StaticValue::StaticTuple(vec![]))
    }
}

struct IsCursedCall;
impl ContractCall for IsCursedCall {
    fn contract_call_signature() -> &'static str {
        "isCursed()"
    }
    fn contract_call_parameters(self) -> ValueToEncode {
        ValueToEncode::Static(StaticValue::StaticTuple(vec![]))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionedOnchainConfig {
    pub version: u32,
    pub block_number: u32,
    pub config: OnchainConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlessProgress {
    pub bless_vote_addrs: Vec<Address>,
    pub accumulated_weight: u16,
    pub blessed: bool,
}

#[derive(Clone)]
pub struct AFNInterface {
    pub address: Address,
    rpc: Arc<Rpc>,
}

impl AFNInterface {
    pub fn create_from_chain_config(rpc: Arc<Rpc>, config: &ChainConfig) -> Result<Self> {
        Ok(AFNInterface {
            address: config.afn_contract,
            rpc,
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
        let data: Bytes = GetConfigDetailsCall.encode_contract_call();
        let (version, block_number, config) = {
            let return_data = self.rpc.call(self.address, data)?;
            <(u32, u32, OnchainConfig)>::abi_decode(return_data)?
        };
        Ok(VersionedOnchainConfig {
            version,
            block_number,
            config,
        })
    }

    pub fn send_vote_to_curse(
        &self,
        tx_sender: &mut TransactionSender,
        curse_id: Bytes32,
    ) -> Result<Bytes32> {
        let data = VoteToCurseCall { curse_id }.encode_contract_call();
        self.send_transaction(tx_sender, data, 200_000u64)
    }

    pub fn send_vote_to_bless(
        &self,
        tx_sender: &mut TransactionSender,
        votes: Vec<TaggedRoot>,
    ) -> Result<Bytes32> {
        let gas = 70_000u64 * votes.len() as u64;
        let data: Bytes = VoteToBlessCall {
            tagged_roots: votes,
        }
        .encode_contract_call();
        self.send_transaction(tx_sender, data, gas)
    }
}
