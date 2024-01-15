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
pub struct Evm2EvmMessageV1_2 {
    pub source_chain_selector: ChainSelector,
    pub sender: Address,
    pub receiver: Address,
    pub sequence_number: u64,
    pub gas_limit: U256,
    pub strict: bool,
    pub nonce: u64,
    pub fee_token: Address,
    pub fee_token_amount: U256,
    pub data: Bytes,
    pub token_amounts: Vec<EVMTokenAmount>,
    pub source_token_data: Vec<Bytes>,
    pub message_id: Bytes32,
}
type Evm2EvmMessageAsTuple = (
    u64,
    Address,
    Address,
    u64,
    U256,
    bool,
    u64,
    Address,
    U256,
    Bytes,
    Vec<EVMTokenAmount>,
    Vec<Bytes>,
    Bytes32,
);

impl AbiTypeToDecode for Evm2EvmMessageV1_2 {
    fn abi_type_to_decode() -> Type {
        Evm2EvmMessageAsTuple::abi_type_to_decode()
    }
}

impl TryFrom<Value> for Evm2EvmMessageV1_2 {
    type Error = AbiDecodeError;
    fn try_from(v: Value) -> Result<Self, Self::Error> {
        let (
            source_chain_selector,
            sender,
            receiver,
            sequence_number,
            gas_limit,
            strict,
            nonce,
            fee_token,
            fee_token_amount,
            data,
            token_amounts,
            source_token_data,
            message_id,
        ) = Evm2EvmMessageAsTuple::try_from(v)?;
        Ok(Self {
            source_chain_selector,
            sender,
            receiver,
            sequence_number,
            gas_limit,
            strict,
            nonce,
            fee_token,
            fee_token_amount,
            data,
            token_amounts,
            source_token_data,
            message_id,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CcipSendRequestedEventV1_2 {
    pub message: Evm2EvmMessageV1_2,
}
type CcipSendRequestedEventV1_2AsTuple = (Evm2EvmMessageV1_2,);

impl AbiTypeToDecode for CcipSendRequestedEventV1_2 {
    fn abi_type_to_decode() -> Type {
        CcipSendRequestedEventV1_2AsTuple::abi_type_to_decode()
    }
}
impl TryFrom<Value> for CcipSendRequestedEventV1_2 {
    type Error = AbiDecodeError;
    fn try_from(v: Value) -> Result<Self, Self::Error> {
        let (message,) = CcipSendRequestedEventV1_2AsTuple::try_from(v)?;
        Ok(Self { message })
    }
}

impl LogSignature for CcipSendRequestedEventV1_2 {
    fn log_signature() -> &'static str {
        "CCIPSendRequested((uint64,address,address,uint64,uint256,bool,uint64,address,uint256,bytes,(address,uint256)[],bytes[],bytes32))"
    }
}

impl UncheckedDecodeLog for CcipSendRequestedEventV1_2 {
    fn unchecked_decode_log(log: EVMLog) -> Result<Self> {
        Ok(Self::abi_decode(log.data)?)
    }
}
impl Hashable for Evm2EvmMessageV1_2 {
    fn hash(&self, metadata: &MessageMetadata) -> Bytes32 {
        Bytes32::from(keccak256(
            (
                Bytes32::from(*LEAF_DOMAIN_SEPARATOR),
                metadata_hash("EVM2EVMMessageHashV2", metadata),
                Bytes32::from(keccak256(
                    (
                        self.sender,
                        self.receiver,
                        self.sequence_number,
                        self.gas_limit,
                        self.strict,
                        self.nonce,
                        self.fee_token,
                        self.fee_token_amount,
                    )
                        .abi_encode()
                        .as_ref(),
                )),
                Bytes32::from(keccak256(self.data.as_ref())),
                Bytes32::from(keccak256(self.token_amounts.clone().abi_encode().as_ref())),
                Bytes32::from(keccak256(
                    self.source_token_data.clone().abi_encode().as_ref(),
                )),
            )
                .abi_encode()
                .as_ref(),
        ))
    }
}
