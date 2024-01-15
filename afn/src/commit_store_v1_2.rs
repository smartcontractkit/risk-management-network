use crate::{
    commit_store_common::{CommitReport, GasPriceUpdate, TokenPriceUpdate},
    common::{LogSignature, UncheckedDecodeLog},
};
use anyhow::Result;
use miniabi::{
    abi_decode::{AbiDecode, AbiDecodeError, AbiTypeToDecode},
    types::{Type, Value},
};
use minieth::rpc::EVMLog;

#[derive(Debug, Eq, Hash, PartialEq, Clone)]
pub struct PriceUpdatesV1_2 {
    token_price_updates: Vec<TokenPriceUpdate>,
    gas_price_updates: Vec<GasPriceUpdate>,
}
type PriceUpdatesV1_2AsTuple = (Vec<TokenPriceUpdate>, Vec<GasPriceUpdate>);
impl AbiTypeToDecode for PriceUpdatesV1_2 {
    fn abi_type_to_decode() -> Type {
        PriceUpdatesV1_2AsTuple::abi_type_to_decode()
    }
}
impl TryFrom<Value> for PriceUpdatesV1_2 {
    type Error = AbiDecodeError;
    fn try_from(v: Value) -> Result<Self, Self::Error> {
        let (token_price_updates, gas_price_updates) = PriceUpdatesV1_2AsTuple::try_from(v)?;
        Ok(Self {
            token_price_updates,
            gas_price_updates,
        })
    }
}

pub type CommitReportV1_2 = CommitReport<PriceUpdatesV1_2>;

#[derive(Debug, Clone)]
pub struct ReportAcceptedEventV1_2 {
    pub report: CommitReportV1_2,
}
type ReportAcceptedEventV1_2AsTuple = (CommitReportV1_2,);

impl AbiTypeToDecode for ReportAcceptedEventV1_2 {
    fn abi_type_to_decode() -> Type {
        ReportAcceptedEventV1_2AsTuple::abi_type_to_decode()
    }
}
impl TryFrom<Value> for ReportAcceptedEventV1_2 {
    type Error = AbiDecodeError;
    fn try_from(v: Value) -> Result<Self, Self::Error> {
        let (report,) = ReportAcceptedEventV1_2AsTuple::try_from(v)?;
        Ok(Self { report })
    }
}

impl LogSignature for ReportAcceptedEventV1_2 {
    fn log_signature() -> &'static str {
        "ReportAccepted((((address,uint224)[],(uint64,uint224)[]),(uint64,uint64),bytes32))"
    }
}

impl UncheckedDecodeLog for ReportAcceptedEventV1_2 {
    fn unchecked_decode_log(log: EVMLog) -> Result<Self> {
        let event = Self::abi_decode(log.data)?;
        Ok(event)
    }
}
