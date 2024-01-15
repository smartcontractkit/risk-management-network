use anyhow::{anyhow, Result};
use miniabi::{abi_encode::AbiEncode, types::Value};
use minieth::{
    bytes::{Bytes, Bytes32},
    keccak::keccak256,
    rpc::EVMLog,
};
use serde::{Deserialize, Serialize};
use std::{
    fmt::{Debug, Display},
    str::FromStr,
};

pub type ChainId = u64;

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy, Serialize, Deserialize, PartialOrd, Ord)]
#[repr(u64)]
pub enum ChainName {
    Arbitrum = 42161,
    Avax = 43114,
    Base = 8453,
    Bsc = 56,
    Ethereum = 1,
    Kroma = 255,
    Optimism = 10,
    Polygon = 137,
    PolygonZkEvm = 1101,
    Scroll = 534352,
    Wemix = 1111,

    ArbitrumGoerli = 421613,
    ArbitrumSepolia = 421614,
    AvaxFuji = 43113,
    BaseGoerli = 84531,
    BaseSepolia = 84532,
    BscTestnet = 97,
    Goerli = 5,
    KromaSepolia = 2358,
    OptimismGoerli = 420,
    OptimismSepolia = 11155420,
    PolygonMumbai = 80001,
    PolygonZkEvmTestnet = 1442,
    ScrollSepolia = 534351,
    Sepolia = 11155111,
    WemixTestnet = 1112,
    ZkSyncGoerli = 280,
}

impl ChainName {
    pub fn chain_id(self) -> ChainId {
        self as u64
    }
}

impl FromStr for ChainName {
    type Err = anyhow::Error;

    fn from_str(input: &str) -> Result<ChainName> {
        match input.to_uppercase().as_str() {
            "GOERLI" => Ok(ChainName::Goerli),
            "AVAXFUJI" => Ok(ChainName::AvaxFuji),
            "OPTIMISMGOERLI" => Ok(ChainName::OptimismGoerli),
            "SEPOLIA" => Ok(ChainName::Sepolia),
            "ARBITRUMGOERLI" => Ok(ChainName::ArbitrumGoerli),
            "POLYGONMUMBAI" => Ok(ChainName::PolygonMumbai),
            _ => Err(anyhow!("unknown ChainName {input:?}")),
        }
    }
}

impl Display for ChainName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, Serialize, Deserialize, PartialOrd, Ord)]
pub struct LaneId {
    pub source_chain_name: ChainName,
    pub dest_chain_name: ChainName,
    pub name: String,
}

impl Display for LaneId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Lane({}->{},{})",
            self.source_chain_name, self.dest_chain_name, self.name
        )
    }
}

fn check_event_topic<T: LogSignature>(log: &EVMLog) -> Result<()> {
    let event_name = T::log_signature();
    let topic = *log
        .topics
        .first()
        .ok_or_else(|| anyhow!("{event_name}: log.topics[0] is needed to check event topic"))?;
    if topic != T::log_topic() {
        Err(anyhow!(
            "{event_name}: log.topics[0] doesn't match with event topic, got {}, expected {}",
            topic,
            T::log_topic()
        ))
    } else {
        Ok(())
    }
}

pub trait LogSignature {
    fn log_signature() -> &'static str;
}

pub trait LogTopic: LogSignature {
    fn log_topic() -> Bytes32;
}

impl<T: LogSignature> LogTopic for T {
    fn log_topic() -> Bytes32 {
        keccak256(T::log_signature().as_bytes()).into()
    }
}

pub trait UncheckedDecodeLog: Sized + LogSignature {
    fn unchecked_decode_log(log: EVMLog) -> Result<Self>;
}

pub trait DecodeLog: UncheckedDecodeLog {
    fn decode_log(log: EVMLog) -> Result<Self>;
}

impl<T: UncheckedDecodeLog> DecodeLog for T {
    fn decode_log(log: EVMLog) -> Result<Self> {
        check_event_topic::<Self>(&log)?;
        Self::unchecked_decode_log(log)
    }
}

pub trait ContractCall {
    fn contract_call_signature() -> &'static str;
    fn contract_call_parameters(self) -> Value;
}

pub trait ContractCallSelector: ContractCall {
    fn contract_call_selector() -> [u8; 4];
}
impl<T: ContractCall> ContractCallSelector for T {
    fn contract_call_selector() -> [u8; 4] {
        let v: [u8; 32] = keccak256(Self::contract_call_signature().as_bytes());
        v[0..4].try_into().unwrap()
    }
}

pub trait EncodeContractCall: ContractCall {
    fn encode_contract_call(self) -> Bytes;
}

impl<T: ContractCall> EncodeContractCall for T {
    fn encode_contract_call(self) -> Bytes {
        let mut selector: Vec<u8> = T::contract_call_selector().into();
        let params: Vec<u8> = self.contract_call_parameters().abi_encode().into();
        selector.extend(params.iter());
        selector.into()
    }
}
