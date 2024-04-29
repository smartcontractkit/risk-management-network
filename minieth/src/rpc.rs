use serde::{de, Deserialize, Serialize, Serializer};
use serde_json::{json, Value};
use std::str::FromStr;
use thiserror::Error;

use crate::bytes::{Address, Bytes, Bytes32};
use crate::json_rpc::{
    Call, CallResponse, JsonRpc, JsonRpcCall, JsonRpcError, JsonRpcNewError, JsonRpcResponseError,
};
use crate::u256::U256;

#[derive(Error, Debug)]
pub enum RpcError {
    #[error("internal error: {0}")]
    InternalError(#[from] JsonRpcError),
    #[error("serde_json failed: {0}")]
    SerdeJsonError(#[from] serde_json::Error),
    #[error("invalid input to rpc call, caught before sending: {0}")]
    InvalidInput(String),
    #[error("response is an error: {0}")]
    ErrorResponse(#[from] JsonRpcResponseError),
    #[error("requested resource was not found")]
    NotFound,
    #[error("unexpected chain id: expected {expected}, actual {actual}")]
    UnexpectedChainId { expected: u64, actual: u64 },
}

fn hex_number(num: u64) -> String {
    format!("{num:#x}")
}

fn serialize_hex_u64<S: Serializer>(
    num: &u64,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.serialize_str(&hex_number(*num))
}

fn deserialize_hex_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: de::Deserializer<'de>,
{
    use serde::de::Error;

    let s: String = de::Deserialize::deserialize(deserializer)?;
    let u256 = U256::from_str(&s).map_err(Error::custom)?;
    u256.try_into().map_err(Error::custom)
}

#[derive(Serialize, Deserialize)]
struct HexU64(
    #[serde(
        deserialize_with = "deserialize_hex_u64",
        serialize_with = "serialize_hex_u64"
    )]
    u64,
);

impl From<HexU64> for u64 {
    fn from(value: HexU64) -> Self {
        value.0
    }
}

impl From<u64> for HexU64 {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    #[serde(deserialize_with = "deserialize_hex_u64")]
    pub number: u64,
    pub hash: Bytes32,
    pub parent_hash: Bytes32,
    #[serde(deserialize_with = "deserialize_hex_u64")]
    pub timestamp: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EVMLog {
    #[serde(
        deserialize_with = "deserialize_hex_u64",
        serialize_with = "serialize_hex_u64"
    )]
    pub log_index: u64,
    pub address: Address,
    pub data: Bytes,
    pub topics: Vec<Bytes32>,
    pub block_hash: Bytes32,
    #[serde(
        deserialize_with = "deserialize_hex_u64",
        serialize_with = "serialize_hex_u64"
    )]
    pub block_number: u64,
}

pub trait BlockRpc: Sync + Send {
    fn get_block(&self, identifier: BlockIdentifier) -> Result<Block, RpcError>;

    fn get_blocks(
        &self,
        identifiers: &[BlockIdentifier],
    ) -> Result<Vec<Result<Block, RpcError>>, RpcError>;
}

#[derive(Debug)]
pub struct Rpc {
    chain_id: u64,
    json_rpc: JsonRpc,
}

#[derive(Debug, Clone, Copy)]
pub enum BlockIdentifier {
    Height(u64),
    Hash(Bytes32),
    Latest,
    Earliest,
    Safe,
    Finalized,
    Pending,
}

impl ToString for BlockIdentifier {
    fn to_string(&self) -> String {
        match self {
            Self::Height(u) => hex_number(*u),
            Self::Hash(h) => h.to_string(),
            Self::Latest => "latest".into(),
            Self::Earliest => "earliest".into(),
            Self::Safe => "safe".into(),
            Self::Finalized => "finalized".into(),
            Self::Pending => "pending".into(),
        }
    }
}
impl Serialize for BlockIdentifier {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

struct GetBlockCall {
    identifier: BlockIdentifier,
}

impl From<GetBlockCall> for Call {
    fn from(val: GetBlockCall) -> Self {
        let method = match val.identifier {
            BlockIdentifier::Hash(_) => "eth_getBlockByHash",
            _ => "eth_getBlockByNumber",
        }
        .into();
        Call {
            method,
            params: vec![json!(val.identifier), json!(false)],
        }
    }
}

struct GetLogsCall {
    from_block: u64,
    to_block: u64,
    addresses: Vec<Address>,
    first_topics: Vec<Bytes32>,
}

impl From<GetLogsCall> for Call {
    fn from(val: GetLogsCall) -> Call {
        Call {
            method: "eth_getLogs".into(),
            params: vec![json!({
                "fromBlock": BlockIdentifier::Height(val.from_block),
                "toBlock": BlockIdentifier::Height(val.to_block),
                "address": val.addresses,
                "topics": [val.first_topics.iter().collect::<Vec<_>>()],
            })],
        }
    }
}

struct CallCall {
    from: Option<Address>,
    to: Address,
    data: Bytes,
    gas: Option<u64>,
}

impl From<CallCall> for Call {
    fn from(val: CallCall) -> Call {
        Call {
            method: "eth_call".into(),
            params: vec![
                json!({
                    "from": val.from,
                    "to": val.to,
                    "data": val.data,
                    "gas": val.gas.map(U256::from),
                }),
                json!(BlockIdentifier::Latest),
            ],
        }
    }
}

struct EstimateGasCall {
    from: Address,
    to: Address,
    data: Bytes,
    block: BlockIdentifier,
}

impl From<EstimateGasCall> for Call {
    fn from(val: EstimateGasCall) -> Call {
        Call {
            method: "eth_estimateGas".into(),
            params: vec![
                json!({
                    "from": val.from,
                    "to": val.to,
                    "data": val.data,
                }),
                json!(val.block),
            ],
        }
    }
}

struct SendRawTransactionCall {
    raw_tx: Bytes,
}

impl From<SendRawTransactionCall> for Call {
    fn from(val: SendRawTransactionCall) -> Self {
        Call {
            method: "eth_sendRawTransaction".into(),
            params: vec![json!(val.raw_tx)],
        }
    }
}

struct GetTransactionCountCall {
    address: Address,
    block: BlockIdentifier,
}

impl From<GetTransactionCountCall> for Call {
    fn from(val: GetTransactionCountCall) -> Self {
        Call {
            method: "eth_getTransactionCount".into(),
            params: vec![json!(val.address), json!(val.block)],
        }
    }
}

pub struct ChainIdCall();

impl From<ChainIdCall> for Call {
    fn from(_: ChainIdCall) -> Self {
        Call {
            method: "eth_chainId".into(),
            params: vec![],
        }
    }
}

pub struct FeeHistoryCall {
    pub block_count: U256,
    pub newest_block: BlockIdentifier,
    pub reward_percentiles: Vec<u64>,
}

impl From<FeeHistoryCall> for Call {
    fn from(val: FeeHistoryCall) -> Self {
        Call {
            method: "eth_feeHistory".into(),
            params: vec![
                json!(val.block_count),
                json!(val.newest_block),
                json!(val.reward_percentiles),
            ],
        }
    }
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseFeePerGasInGetBlockResponse {
    pub base_fee_per_gas: U256,
}

pub struct GasPriceCall();
impl From<GasPriceCall> for Call {
    fn from(_: GasPriceCall) -> Self {
        Call {
            method: "eth_gasPrice".into(),
            params: vec![],
        }
    }
}

fn parse_get_block_response(call_response: CallResponse) -> Result<Block, RpcError> {
    match call_response {
        Ok(Value::Null) => Err(RpcError::NotFound),
        Err(e) => Err(RpcError::ErrorResponse(e)),
        Ok(v) => {
            let block: Block = serde_json::from_value(v)?;
            Ok(block)
        }
    }
}

fn parse_get_logs_response(call_response: CallResponse) -> Result<Vec<EVMLog>, RpcError> {
    match call_response {
        Ok(Value::Null) => Err(RpcError::NotFound),
        Err(e) => Err(RpcError::ErrorResponse(e)),
        Ok(v) => {
            let mut logs: Vec<EVMLog> = serde_json::from_value(v)?;

            logs.sort_by_key(|log| (log.block_number, log.log_index));

            Ok(logs)
        }
    }
}

impl BlockRpc for Rpc {
    fn get_block(&self, identifier: BlockIdentifier) -> Result<Block, RpcError> {
        self.get_blocks(&[identifier])?
            .pop()
            .ok_or(JsonRpcError::IncompleteBatch)?
    }

    fn get_blocks(
        &self,
        identifiers: &[BlockIdentifier],
    ) -> Result<Vec<Result<Block, RpcError>>, RpcError> {
        Ok(self
            .batch_call(
                identifiers
                    .iter()
                    .map(|&identifier| GetBlockCall { identifier }.into())
                    .collect::<Vec<_>>(),
            )?
            .into_iter()
            .map(parse_get_block_response)
            .collect::<Vec<_>>())
    }
}

impl Rpc {
    pub fn new_with_multiple_urls(chain_id: u64, urls: &[String]) -> Result<Self, JsonRpcNewError> {
        Ok(Self {
            chain_id,
            json_rpc: JsonRpc::new_many(urls)?,
        })
    }

    pub fn new(chain_id: u64, url: &str) -> Result<Self, JsonRpcNewError> {
        Self::new_with_multiple_urls(chain_id, &[url.to_owned()])
    }

    fn batch_call(&self, mut calls: Vec<Call>) -> Result<Vec<CallResponse>, RpcError> {
        calls.push(ChainIdCall().into());
        let mut batch_responses = self.json_rpc.call(calls.as_slice())?;
        let chain_id_response = batch_responses.pop();
        match chain_id_response {
            None => Err(JsonRpcError::IncompleteBatch.into()),
            Some(CallResponse::Err(err)) => Err(JsonRpcError::ErrorResponse(err).into()),
            Some(CallResponse::Ok(chain_id)) => {
                let actual_chain_id: u64 = serde_json::from_value::<HexU64>(chain_id)?.into();
                if self.chain_id == actual_chain_id {
                    Ok(batch_responses)
                } else {
                    Err(RpcError::UnexpectedChainId {
                        expected: self.chain_id,
                        actual: actual_chain_id,
                    })
                }
            }
        }
    }

    fn call_one(&self, call: impl Into<Call>) -> Result<Value, RpcError> {
        let v = self
            .batch_call(vec![call.into()])?
            .into_iter()
            .next()
            .unwrap()?;
        if v == Value::Null {
            Err(RpcError::NotFound)
        } else {
            Ok(v)
        }
    }

    pub fn call(&self, to: Address, data: Bytes) -> Result<Bytes, RpcError> {
        Ok(serde_json::from_value(self.call_one(CallCall {
            from: None,
            to,
            data,
            gas: None,
        })?)?)
    }

    pub fn get_logs(
        &self,
        from_block: u64,
        to_block: u64,
        addresses: &[Address],
        first_topics: &[Bytes32],
    ) -> Result<Vec<EVMLog>, RpcError> {
        if from_block > to_block {
            return Err(RpcError::InvalidInput(format!(
                "got from_block {} and to_block {}",
                from_block, to_block
            )));
        }
        let batch_responses: Result<[_; 3], _> = self
            .batch_call(vec![
                GetBlockCall {
                    identifier: BlockIdentifier::Height(from_block),
                }
                .into(),
                GetBlockCall {
                    identifier: BlockIdentifier::Height(to_block),
                }
                .into(),
                GetLogsCall {
                    from_block,
                    to_block,
                    addresses: addresses.to_vec(),
                    first_topics: first_topics.to_vec(),
                }
                .into(),
            ])?
            .try_into();
        match batch_responses {
            Ok([from_block_response, to_block_response, get_logs_response]) => {
                match (
                    parse_get_block_response(from_block_response),
                    parse_get_block_response(to_block_response),
                    parse_get_logs_response(get_logs_response),
                ) {
                    (_, _, Err(e)) | (Err(e), _, _) | (_, Err(e), _) => Err(e),
                    (Ok(_), Ok(_), Ok(logs)) => Ok(logs),
                }
            }
            Err(_) => Err(JsonRpcError::IncompleteBatch.into()),
        }
    }

    pub fn send_raw_transaction(&self, raw_tx: Bytes) -> Result<Bytes32, RpcError> {
        Ok(serde_json::from_value(
            self.call_one(SendRawTransactionCall { raw_tx })?,
        )?)
    }

    pub fn get_transaction_count(
        &self,
        address: Address,
        block: BlockIdentifier,
    ) -> Result<u64, RpcError> {
        Ok(serde_json::from_value::<HexU64>(
            self.call_one(GetTransactionCountCall { address, block })?,
        )?
        .into())
    }

    pub fn get_latest_block_base_fee_per_gas_wei(&self) -> Result<U256, RpcError> {
        Ok(
            serde_json::from_value::<BaseFeePerGasInGetBlockResponse>(self.call_one(
                GetBlockCall {
                    identifier: BlockIdentifier::Latest,
                },
            )?)?
            .base_fee_per_gas,
        )
    }

    pub fn get_gas_price_wei(&self) -> Result<U256, RpcError> {
        Ok(serde_json::from_value(self.call_one(GasPriceCall())?)?)
    }

    pub(crate) fn estimate_gas(
        &self,
        from: Address,
        to: Address,
        data: Bytes,
        block: BlockIdentifier,
    ) -> Result<u64, RpcError> {
        let hexu64: HexU64 = serde_json::from_value(self.call_one(EstimateGasCall {
            from,
            to,
            data,
            block,
        })?)?;
        Ok(hexu64.into())
    }
}
