use crate::{
    commit_store_common::{CommitReport, TokenPriceUpdate},
    common::{LogSignature, UncheckedDecodeLog},
};
use anyhow::Result;
use miniabi::{
    abi_decode::{AbiDecode, AbiDecodeError, AbiTypeToDecode},
    types::{Type, Value},
};
use minieth::{rpc::EVMLog, u256::U256};

#[derive(Debug, Eq, Hash, PartialEq, Clone)]
pub struct PriceUpdatesV1_0 {
    token_price_updates: Vec<TokenPriceUpdate>,
    dest_chain_selector: u64,
    usd_per_unit_gas: U256,
}
type PriceUpdatesV1_0AsTuple = (Vec<TokenPriceUpdate>, u64, U256);
impl AbiTypeToDecode for PriceUpdatesV1_0 {
    fn abi_type_to_decode() -> Type {
        PriceUpdatesV1_0AsTuple::abi_type_to_decode()
    }
}
impl TryFrom<Value> for PriceUpdatesV1_0 {
    type Error = AbiDecodeError;
    fn try_from(v: Value) -> Result<Self, Self::Error> {
        let (token_price_updates, dest_chain_selector, usd_per_unit_gas) =
            PriceUpdatesV1_0AsTuple::try_from(v)?;
        Ok(Self {
            token_price_updates,
            dest_chain_selector,
            usd_per_unit_gas,
        })
    }
}

pub type CommitReportV1_0 = CommitReport<PriceUpdatesV1_0>;

#[derive(Debug, Clone)]
pub struct ReportAcceptedEventV1_0 {
    pub report: CommitReportV1_0,
}
type ReportAcceptedEventV1_0AsTuple = (CommitReportV1_0,);

impl AbiTypeToDecode for ReportAcceptedEventV1_0 {
    fn abi_type_to_decode() -> Type {
        ReportAcceptedEventV1_0AsTuple::abi_type_to_decode()
    }
}
impl TryFrom<Value> for ReportAcceptedEventV1_0 {
    type Error = AbiDecodeError;
    fn try_from(v: Value) -> Result<Self, Self::Error> {
        let (report,) = ReportAcceptedEventV1_0AsTuple::try_from(v)?;
        Ok(Self { report })
    }
}

impl LogSignature for ReportAcceptedEventV1_0 {
    fn log_signature() -> &'static str {
        "ReportAccepted((((address,uint192)[],uint64,uint192),(uint64,uint64),bytes32))"
    }
}

impl UncheckedDecodeLog for ReportAcceptedEventV1_0 {
    fn unchecked_decode_log(log: EVMLog) -> Result<Self> {
        let event = Self::abi_decode(log.data)?;
        Ok(event)
    }
}
