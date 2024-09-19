use crate::chain_selector::ChainSelector;
use crate::onramp_traits::OnRampReader;
use crate::{
    common::{LogSignature, UncheckedDecodeLog},
    merkle::root_from_hashed_leafs,
};
use anyhow::{anyhow, Result};
use miniabi::{
    abi_decode::{AbiDecode, AbiDecodeError, AbiTypeToDecode},
    types::{Type, Value},
};
use minieth::rpc::EVMLog;
use minieth::{
    bytes::{Address, Bytes32},
    u256::U256,
};

#[derive(Debug, Eq, Hash, PartialEq, Clone, Copy)]
pub struct Interval {
    pub min: u64,
    pub max: u64,
}
type IntervalAsTuple = (u64, u64);
impl AbiTypeToDecode for Interval {
    fn abi_type_to_decode() -> Type {
        IntervalAsTuple::abi_type_to_decode()
    }
}

impl TryFrom<Value> for Interval {
    type Error = AbiDecodeError;
    fn try_from(v: Value) -> Result<Self, Self::Error> {
        let (min, max) = IntervalAsTuple::try_from(v)?;
        Ok(Self { min, max })
    }
}

#[derive(Debug, Eq, Hash, PartialEq, Clone)]
pub struct GenericCommitReport<PriceUpdates: AbiTypeToDecode> {
    pub price_updates: PriceUpdates,
    pub interval: Interval,
    pub merkle_root: Bytes32,
}

impl<T: AbiTypeToDecode> From<GenericCommitReport<T>> for RootWithInterval {
    fn from(commit_report: GenericCommitReport<T>) -> Self {
        RootWithInterval {
            root: commit_report.merkle_root,
            interval: commit_report.interval,
        }
    }
}

type CommitReportAsTuple<PriceUpdates> = (PriceUpdates, Interval, Bytes32);

impl<PriceUpdates: AbiTypeToDecode> AbiTypeToDecode for GenericCommitReport<PriceUpdates> {
    fn abi_type_to_decode() -> Type {
        CommitReportAsTuple::<PriceUpdates>::abi_type_to_decode()
    }
}
impl<PriceUpdates: AbiTypeToDecode> TryFrom<Value> for GenericCommitReport<PriceUpdates>
where
    CommitReportAsTuple<PriceUpdates>: TryFrom<Value, Error = AbiDecodeError>,
{
    type Error = AbiDecodeError;
    fn try_from(v: Value) -> Result<Self, Self::Error> {
        let (price_updates, interval, merkle_root) =
            CommitReportAsTuple::<PriceUpdates>::try_from(v)?;
        Ok(Self {
            price_updates,
            interval,
            merkle_root,
        })
    }
}

#[derive(Debug, Eq, Hash, PartialEq, Clone)]
pub struct TokenPriceUpdate {
    pub source_token: Address,
    pub usd_per_token: U256,
}
type TokenPriceUpdateAsTuple = (Address, U256);
impl AbiTypeToDecode for TokenPriceUpdate {
    fn abi_type_to_decode() -> Type {
        TokenPriceUpdateAsTuple::abi_type_to_decode()
    }
}
impl TryFrom<Value> for TokenPriceUpdate {
    type Error = AbiDecodeError;
    fn try_from(v: Value) -> Result<Self, Self::Error> {
        let (source_token, usd_per_token) = TokenPriceUpdateAsTuple::try_from(v)?;
        Ok(Self {
            source_token,
            usd_per_token,
        })
    }
}

#[derive(Debug, Eq, Hash, PartialEq, Clone)]
pub struct GasPriceUpdate {
    pub dest_chain_selector: u64,
    pub usd_per_unit_gas: U256,
}

type GasPriceUpdateAsTuple = (u64, U256);
impl AbiTypeToDecode for GasPriceUpdate {
    fn abi_type_to_decode() -> Type {
        GasPriceUpdateAsTuple::abi_type_to_decode()
    }
}
impl TryFrom<Value> for GasPriceUpdate {
    type Error = AbiDecodeError;
    fn try_from(v: Value) -> Result<Self, Self::Error> {
        let (dest_chain_selector, usd_per_unit_gas) = GasPriceUpdateAsTuple::try_from(v)?;
        Ok(Self {
            dest_chain_selector,
            usd_per_unit_gas,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RootWithInterval {
    pub root: Bytes32,
    pub interval: Interval,
}

pub fn verify_root_with_interval(
    ri: &RootWithInterval,
    reader: &impl OnRampReader,
) -> Result<bool> {
    Ok(ri.root == compute_merkle_root(&ri.interval, reader)?)
}

fn is_valid_interval(interval: &Interval) -> bool {
    interval.max >= interval.min
}

fn compute_merkle_root(interval: &Interval, reader: &impl OnRampReader) -> Result<Bytes32> {
    if !is_valid_interval(interval) {
        return Err(anyhow!(
            "compute_merkle_root: invalid interval {:?}",
            interval
        ));
    }
    let leaves: Vec<Bytes32> = reader.get_message_hashes(interval).ok_or_else(|| {
        anyhow!(
            "compute_merkle_root: failed to get message hashes for interval: {:?}",
            interval
        )
    })?;
    let expected_num_leaves = interval.max - interval.min + 1;
    if leaves.len() as u64 != expected_num_leaves {
        return Err(anyhow!(
                "compute_merkle_root: incorrect number of messages in the interval {:?}, expected {}, got {}.",
                interval,
                expected_num_leaves,
                leaves.len(),
            ));
    }
    let root = root_from_hashed_leafs(&leaves).ok_or_else(|| {
        anyhow!(
            "compute_merkle_root: failed to compute root from leaves {:?}",
            leaves
        )
    })?;
    Ok(root)
}

#[derive(Debug, Eq, Hash, PartialEq, Clone)]
pub struct PriceUpdates {
    pub token_price_updates: Vec<TokenPriceUpdate>,
    pub gas_price_updates: Vec<GasPriceUpdate>,
}
type PriceUpdatesAsTuple = (Vec<TokenPriceUpdate>, Vec<GasPriceUpdate>);
impl AbiTypeToDecode for PriceUpdates {
    fn abi_type_to_decode() -> Type {
        PriceUpdatesAsTuple::abi_type_to_decode()
    }
}
impl TryFrom<Value> for PriceUpdates {
    type Error = AbiDecodeError;
    fn try_from(v: Value) -> Result<Self, Self::Error> {
        let (token_price_updates, gas_price_updates) = PriceUpdatesAsTuple::try_from(v)?;
        Ok(Self {
            token_price_updates,
            gas_price_updates,
        })
    }
}

pub type CommitReport = GenericCommitReport<PriceUpdates>;

#[derive(Debug, Clone)]
pub struct ReportAcceptedEvent {
    pub report: CommitReport,
}
type ReportAcceptedEventAsTuple = (CommitReport,);

impl AbiTypeToDecode for ReportAcceptedEvent {
    fn abi_type_to_decode() -> Type {
        ReportAcceptedEventAsTuple::abi_type_to_decode()
    }
}
impl TryFrom<Value> for ReportAcceptedEvent {
    type Error = AbiDecodeError;
    fn try_from(v: Value) -> Result<Self, Self::Error> {
        let (report,) = ReportAcceptedEventAsTuple::try_from(v)?;
        Ok(Self { report })
    }
}

impl LogSignature for ReportAcceptedEvent {
    fn log_signature() -> &'static str {
        "ReportAccepted((((address,uint224)[],(uint64,uint224)[]),(uint64,uint64),bytes32))"
    }
}

impl UncheckedDecodeLog for ReportAcceptedEvent {
    fn unchecked_decode_log(log: EVMLog) -> Result<Self> {
        let event = Self::abi_decode(log.data)?;
        Ok(event)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct StaticConfig {
    pub chain_selector: ChainSelector,
    pub src_chain_selector: ChainSelector,
    pub on_ramp: Address,
}

type StaticConfigAsTuple = (u64, u64, Address, Address);

impl AbiTypeToDecode for StaticConfig {
    fn abi_type_to_decode() -> Type {
        StaticConfigAsTuple::abi_type_to_decode()
    }
}

impl TryFrom<Value> for StaticConfig {
    type Error = AbiDecodeError;
    fn try_from(v: Value) -> Result<Self, Self::Error> {
        let (chain_selector, src_chain_selector, on_ramp, _arm_proxy) =
            StaticConfigAsTuple::try_from(v)?;
        Ok(Self {
            chain_selector,
            src_chain_selector,
            on_ramp,
        })
    }
}
