use crate::{
    chain_selector::ChainSelector,
    common::{LogSignature, UncheckedDecodeLog},
    hashable::{hash_abi_encodables, metadata_hash, Hashable, MessageMetadata},
};
use minieth::{
    bytes::{Address, Bytes, Bytes32},
    keccak::{keccak256, Hasher, Keccak},
    rpc::EVMLog,
    u256::U256,
};

use miniabi::{
    abi_decode::{AbiDecode, AbiDecodeError, AbiTypeToDecode, DecodedValue, TypeToDecode},
    abi_encode::{AbiEncode, StaticValue, ValueToEncode},
};

use crate::merkle::LEAF_DOMAIN_SEPARATOR;

use anyhow::Result;

#[derive(Clone, Debug, Copy, PartialEq)]
pub struct EVMTokenAmount {
    pub token: Address,
    pub amount: U256,
}
type EVMTokenAmountAsTuple = (Address, U256);

impl AbiEncode for EVMTokenAmount {
    fn abi_encode_into(&self, bytes: &mut Vec<u8>) {
        let value_to_encode = ValueToEncode::Static(StaticValue::StaticTuple(vec![
            StaticValue::Address(self.token),
            StaticValue::U256(self.amount),
        ]));
        value_to_encode.abi_encode_into(bytes)
    }
}
impl AbiTypeToDecode for EVMTokenAmount {
    fn abi_type_to_decode() -> TypeToDecode {
        EVMTokenAmountAsTuple::abi_type_to_decode()
    }
}

impl TryFrom<DecodedValue> for EVMTokenAmount {
    type Error = AbiDecodeError;
    fn try_from(v: DecodedValue) -> Result<Self, Self::Error> {
        let (token, amount) = EVMTokenAmountAsTuple::try_from(v)?;
        Ok(Self { token, amount })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Evm2EvmMessage {
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
type Evm2EvmMessageAsTuple = (
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

impl AbiTypeToDecode for Evm2EvmMessage {
    fn abi_type_to_decode() -> TypeToDecode {
        Evm2EvmMessageAsTuple::abi_type_to_decode()
    }
}

impl TryFrom<DecodedValue> for Evm2EvmMessage {
    type Error = AbiDecodeError;
    fn try_from(v: DecodedValue) -> Result<Self, Self::Error> {
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
        ) = Evm2EvmMessageAsTuple::try_from(v)?;
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
pub struct CcipsendRequestedEvent {
    pub message: Evm2EvmMessage,
}
type CcipsendRequestedEventAsTuple = (Evm2EvmMessage,);

impl AbiTypeToDecode for CcipsendRequestedEvent {
    fn abi_type_to_decode() -> TypeToDecode {
        CcipsendRequestedEventAsTuple::abi_type_to_decode()
    }
}
impl TryFrom<DecodedValue> for CcipsendRequestedEvent {
    type Error = AbiDecodeError;
    fn try_from(v: DecodedValue) -> Result<Self, Self::Error> {
        let (message,) = CcipsendRequestedEventAsTuple::try_from(v)?;
        Ok(Self { message })
    }
}

impl LogSignature for CcipsendRequestedEvent {
    fn log_signature() -> &'static str {
        "CCIPSendRequested((uint64,uint64,uint256,address,uint64,uint256,bool,address,bytes,(address,uint256)[],address,bytes32))"
    }
}

impl UncheckedDecodeLog for CcipsendRequestedEvent {
    fn unchecked_decode_log(log: EVMLog) -> Result<Self> {
        let event = Self::abi_decode(log.data)?;
        Ok(event)
    }
}

impl Hashable for Evm2EvmMessage {
    fn hash(&self, metadata: &MessageMetadata) -> Bytes32 {
        let mut output = [0u8; 32];
        let mut hasher = Keccak::v256();
        hasher.update(LEAF_DOMAIN_SEPARATOR);
        hasher.update(metadata_hash("EVM2EVMMessageEvent", metadata).as_slice());
        hasher.update(self.sequence_number.abi_encode().as_ref());
        hasher.update(self.nonce.abi_encode().as_ref());
        hasher.update(self.sender.abi_encode().as_ref());
        hasher.update(self.receiver.abi_encode().as_ref());
        hasher.update(&keccak256(self.data.as_ref()));
        hasher.update(hash_abi_encodables(&self.token_amounts).as_slice());
        hasher.update(self.gas_limit.abi_encode().as_ref());
        hasher.update(self.strict.abi_encode().as_ref());
        hasher.update(self.fee_token.abi_encode().as_ref());
        hasher.update(self.fee_token_amount.abi_encode().as_ref());
        hasher.finalize(&mut output);
        Bytes32::from(output)
    }
}
