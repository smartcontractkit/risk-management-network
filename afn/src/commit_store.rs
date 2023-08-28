use crate::{
    commit_report::CommitReport,
    common::{LogSignature, UncheckedDecodeLog},
};
use anyhow::Result;
use miniabi::abi_decode::{AbiDecode, AbiDecodeError, AbiTypeToDecode, DecodedValue, TypeToDecode};
use minieth::{
    bytes::{Address, Bytes32},
    rpc::EVMLog,
    u256::U256,
};

#[derive(Debug, Eq, Hash, PartialEq, Clone)]
struct TokenPriceUpdate {
    source_token: Address,
    usd_per_token: U256,
}
type TokenPriceUpdateAsTuple = (Address, U256);
impl AbiTypeToDecode for TokenPriceUpdate {
    fn abi_type_to_decode() -> TypeToDecode {
        TokenPriceUpdateAsTuple::abi_type_to_decode()
    }
}
impl TryFrom<DecodedValue> for TokenPriceUpdate {
    type Error = AbiDecodeError;
    fn try_from(v: DecodedValue) -> Result<Self, Self::Error> {
        let (source_token, usd_per_token) = TokenPriceUpdateAsTuple::try_from(v)?;
        Ok(Self {
            source_token,
            usd_per_token,
        })
    }
}

#[derive(Debug, Eq, Hash, PartialEq, Clone)]
pub struct PriceUpdates {
    token_price_updates: Vec<TokenPriceUpdate>,
    dest_chain_selector: u64,
    usd_per_unit_gas: U256,
}
type PriceUpdatesAsTuple = (Vec<TokenPriceUpdate>, u64, U256);
impl AbiTypeToDecode for PriceUpdates {
    fn abi_type_to_decode() -> TypeToDecode {
        PriceUpdatesAsTuple::abi_type_to_decode()
    }
}
impl TryFrom<DecodedValue> for PriceUpdates {
    type Error = AbiDecodeError;
    fn try_from(v: DecodedValue) -> Result<Self, Self::Error> {
        let (token_price_updates, dest_chain_selector, usd_per_unit_gas) =
            PriceUpdatesAsTuple::try_from(v)?;
        Ok(Self {
            token_price_updates,
            dest_chain_selector,
            usd_per_unit_gas,
        })
    }
}

#[derive(Debug, Eq, Hash, PartialEq, Clone, Copy)]
pub struct Interval {
    pub min: u64,
    pub max: u64,
}
type IntervalAsTuple = (u64, u64);
impl AbiTypeToDecode for Interval {
    fn abi_type_to_decode() -> TypeToDecode {
        IntervalAsTuple::abi_type_to_decode()
    }
}

impl TryFrom<DecodedValue> for Interval {
    type Error = AbiDecodeError;
    fn try_from(v: DecodedValue) -> Result<Self, Self::Error> {
        let (min, max) = IntervalAsTuple::try_from(v)?;
        Ok(Self { min, max })
    }
}

type CommitReportAsTuple = (PriceUpdates, Interval, Bytes32);

impl AbiTypeToDecode for CommitReport {
    fn abi_type_to_decode() -> TypeToDecode {
        CommitReportAsTuple::abi_type_to_decode()
    }
}
impl TryFrom<DecodedValue> for CommitReport {
    type Error = AbiDecodeError;
    fn try_from(v: DecodedValue) -> Result<Self, Self::Error> {
        let (price_updates, interval, merkle_root) = CommitReportAsTuple::try_from(v)?;
        Ok(Self {
            price_updates,
            interval,
            merkle_root,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ReportAcceptedEvent {
    pub report: CommitReport,
}
type ReportAcceptedEventAsTuple = (CommitReport,);

impl AbiTypeToDecode for ReportAcceptedEvent {
    fn abi_type_to_decode() -> TypeToDecode {
        ReportAcceptedEventAsTuple::abi_type_to_decode()
    }
}
impl TryFrom<DecodedValue> for ReportAcceptedEvent {
    type Error = AbiDecodeError;
    fn try_from(v: DecodedValue) -> Result<Self, Self::Error> {
        let (report,) = ReportAcceptedEventAsTuple::try_from(v)?;
        Ok(Self { report })
    }
}

impl LogSignature for ReportAcceptedEvent {
    fn log_signature() -> &'static str {
        "ReportAccepted((((address,uint192)[],uint64,uint192),(uint64,uint64),bytes32))"
    }
}

impl UncheckedDecodeLog for ReportAcceptedEvent {
    fn unchecked_decode_log(log: EVMLog) -> Result<Self> {
        let event = Self::abi_decode(log.data)?;
        Ok(event)
    }
}
