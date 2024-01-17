use crate::{
    chain_selector::ChainSelector,
    common::{LogSignature, UncheckedDecodeLog},
    evm_common_types::EVMTokenAmount,
    hashable::{metadata_hash, Hashable, MessageMetadata},
};
use minieth::{
    bytes::{Address, Bytes, Bytes32},
    keccak::keccak256,
    rpc::EVMLog,
    u256::U256,
};

use miniabi::{
    abi_decode::{AbiDecode, AbiDecodeError, AbiTypeToDecode},
    abi_encode::AbiEncode,
    types::{Type, Value},
};

use crate::merkle::LEAF_DOMAIN_SEPARATOR;

use anyhow::Result;

#[derive(Clone, Debug, PartialEq)]
pub struct Evm2EvmMessageV1_0 {
    pub source_chain_selector: ChainSelector,
    pub sequence_number: u64,
    pub fee_token_amount: U256,
    pub sender: Address,
    pub nonce: u64,
    pub gas_limit: U256,
    pub strict: bool,
    pub receiver: Address,
    pub data: Bytes,
    pub token_amounts: Vec<EVMTokenAmount>,
    pub fee_token: Address,
    pub message_id: Bytes32,
}
type Evm2EvmMessageV1_0AsTuple = (
    u64,
    u64,
    U256,
    Address,
    u64,
    U256,
    bool,
    Address,
    Bytes,
    Vec<EVMTokenAmount>,
    Address,
    Bytes32,
);

impl AbiTypeToDecode for Evm2EvmMessageV1_0 {
    fn abi_type_to_decode() -> Type {
        Evm2EvmMessageV1_0AsTuple::abi_type_to_decode()
    }
}

impl TryFrom<Value> for Evm2EvmMessageV1_0 {
    type Error = AbiDecodeError;
    fn try_from(v: Value) -> Result<Self, Self::Error> {
        let (
            source_chain_selector,
            sequence_number,
            fee_token_amount,
            sender,
            nonce,
            gas_limit,
            strict,
            receiver,
            data,
            token_amounts,
            fee_token,
            message_id,
        ) = Evm2EvmMessageV1_0AsTuple::try_from(v)?;
        Ok(Self {
            source_chain_selector,
            sequence_number,
            fee_token_amount,
            sender,
            nonce,
            gas_limit,
            strict,
            receiver,
            data,
            token_amounts,
            fee_token,
            message_id,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CcipSendRequestedEventV1_0 {
    pub message: Evm2EvmMessageV1_0,
}
type CcipSendRequestedEventV1_0AsTuple = (Evm2EvmMessageV1_0,);

impl AbiTypeToDecode for CcipSendRequestedEventV1_0 {
    fn abi_type_to_decode() -> Type {
        CcipSendRequestedEventV1_0AsTuple::abi_type_to_decode()
    }
}
impl TryFrom<Value> for CcipSendRequestedEventV1_0 {
    type Error = AbiDecodeError;
    fn try_from(v: Value) -> Result<Self, Self::Error> {
        let (message,) = CcipSendRequestedEventV1_0AsTuple::try_from(v)?;
        Ok(Self { message })
    }
}

impl LogSignature for CcipSendRequestedEventV1_0 {
    fn log_signature() -> &'static str {
        "CCIPSendRequested((uint64,uint64,uint256,address,uint64,uint256,bool,address,bytes,(address,uint256)[],address,bytes32))"
    }
}

impl UncheckedDecodeLog for CcipSendRequestedEventV1_0 {
    fn unchecked_decode_log(log: EVMLog) -> Result<Self> {
        let event = Self::abi_decode(log.data)?;
        Ok(event)
    }
}

impl Hashable for Evm2EvmMessageV1_0 {
    fn hash(&self, metadata: &MessageMetadata) -> Bytes32 {
        Bytes32::from(keccak256(
            (
                Bytes32::from(*LEAF_DOMAIN_SEPARATOR),
                metadata_hash("EVM2EVMMessageEvent", metadata),
                self.sequence_number,
                self.nonce,
                self.sender,
                self.receiver,
                Bytes32::from(keccak256(self.data.as_ref())),
                Bytes32::from(keccak256(self.token_amounts.clone().abi_encode().as_ref())),
                self.gas_limit,
                self.strict,
                self.fee_token,
                self.fee_token_amount,
            )
                .abi_encode()
                .as_ref(),
        ))
    }
}
