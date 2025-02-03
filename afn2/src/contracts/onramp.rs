use anyhow::{anyhow, Context};
use minieth::bytes::Bytes4;

use crate::contracts::prelude::*;

#[derive(Debug)]
pub struct CcipMessageSent {
    pub dest_chain_selector: ChainSelector,
    pub sequence_number: u64,
    pub evm2any_ramp_message: Evm2AnyRampMessage,
}

impl LogSignature for CcipMessageSent {
    fn log_signature() -> &'static str {
        "CCIPMessageSent(uint64,uint64,((bytes32,uint64,uint64,uint64,uint64),address,bytes,bytes,bytes,address,uint256,uint256,(address,bytes,bytes,uint256,bytes)[]))"
    }
}

impl UncheckedDecodeLog for CcipMessageSent {
    fn unchecked_decode_log(log: EVMLog) -> Result<Self> {
        let dest_chain_selector = u64::abi_decode(
            log.topics
                .get(1)
                .context("missing topic[1] with dest_chain_selector")?,
        )?;
        let sequence_number = u64::abi_decode(
            log.topics
                .get(2)
                .context("missing topic[2] with sequence_number")?,
        )?;
        let (evm2any_ramp_message,) = <(Evm2AnyRampMessage,)>::abi_decode(log.data)?;
        Ok(CcipMessageSent {
            dest_chain_selector,
            sequence_number,
            evm2any_ramp_message,
        })
    }
}

#[derive(Debug)]
pub struct Evm2AnyRampMessage {
    header: RampMessageHeader,
    sender: Address,
    data: Bytes,
    receiver: Bytes,
    extra_args: Bytes,
    fee_token: Address,
    fee_token_amount: U256,
    fee_value_juels: U256,
    token_amounts: Vec<Evm2AnyTokenTransfer>,
}

impl Evm2AnyRampMessage {
    pub fn number(&self) -> u64 {
        self.header.sequence_number
    }
}

type Evm2AnyRampMessageAsTuple = (
    RampMessageHeader,
    Address,
    Bytes,
    Bytes,
    Bytes,
    Address,
    U256,
    U256,
    Vec<Evm2AnyTokenTransfer>,
);

impl AbiTypeToDecode for Evm2AnyRampMessage {
    fn abi_type_to_decode() -> Type {
        Evm2AnyRampMessageAsTuple::abi_type_to_decode()
    }
}

impl TryFrom<Value> for Evm2AnyRampMessage {
    type Error = AbiDecodeError;
    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let (
            header,
            sender,
            data,
            receiver,
            extra_args,
            fee_token,
            fee_token_amount,
            fee_value_juels,
            token_amounts,
        ) = Evm2AnyRampMessageAsTuple::try_from(value)?;
        Ok(Evm2AnyRampMessage {
            header,
            sender,
            data,
            receiver,
            extra_args,
            fee_token,
            fee_token_amount,
            fee_value_juels,
            token_amounts,
        })
    }
}

fn evm_extra_args_v2_tag() -> Bytes4 {
    Bytes4::from([0x18, 0x1d, 0xcf, 0x10])
}

struct EvmExtraArgsV2 {
    pub gas_limit: U256,
    pub allow_out_of_order_execution: bool,
}

type EvmExtraArgsV2AsTuple = (U256, bool);

impl AbiTypeToDecode for EvmExtraArgsV2 {
    fn abi_type_to_decode() -> Type {
        EvmExtraArgsV2AsTuple::abi_type_to_decode()
    }
}

impl TryFrom<Value> for EvmExtraArgsV2 {
    type Error = AbiDecodeError;
    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let (gas_limit, allow_out_of_order_execution) = EvmExtraArgsV2AsTuple::try_from(value)?;
        Ok(EvmExtraArgsV2 {
            gas_limit,
            allow_out_of_order_execution,
        })
    }
}

impl EvmExtraArgsV2 {
    fn abi_decode_with_selector(enc_with_selector: impl AsRef<[u8]>) -> Result<Self> {
        let enc_with_selector = enc_with_selector.as_ref();
        let expected_selector = evm_extra_args_v2_tag();
        if enc_with_selector.len() < expected_selector.len() {
            return Err(anyhow!("no selector"));
        }
        let (actual_selector, enc) = enc_with_selector.split_at(expected_selector.len());
        if actual_selector != expected_selector.as_ref() {
            let expected_selector = expected_selector.as_ref();
            return Err(anyhow!(
                "unexpected selector {:?}, expected {:?}",
                actual_selector,
                expected_selector
            ));
        }
        Ok(Self::abi_decode(enc)?)
    }
}

#[derive(Debug)]
struct RampMessageHeader {
    message_id: Bytes32,
    source_chain_selector: ChainSelector,
    dest_chain_selector: ChainSelector,
    sequence_number: u64,
    nonce: u64,
}

type RampMessageHeaderAsTuple = (Bytes32, u64, u64, u64, u64);

impl AbiTypeToDecode for RampMessageHeader {
    fn abi_type_to_decode() -> Type {
        RampMessageHeaderAsTuple::abi_type_to_decode()
    }
}

impl TryFrom<Value> for RampMessageHeader {
    type Error = AbiDecodeError;
    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let (message_id, source_chain_selector, dest_chain_selector, sequence_number, nonce) =
            RampMessageHeaderAsTuple::try_from(value)?;
        Ok(RampMessageHeader {
            message_id,
            source_chain_selector,
            dest_chain_selector,
            sequence_number,
            nonce,
        })
    }
}

#[derive(Debug)]
pub struct Evm2AnyTokenTransfer {
    source_pool_address: Address,
    dest_token_address: Bytes,
    extra_data: Bytes,
    amount: U256,
    dest_exec_data: Bytes,
}

type Evm2AnyTokenTransferAsTuple = (Address, Bytes, Bytes, U256, Bytes);

impl AbiTypeToDecode for Evm2AnyTokenTransfer {
    fn abi_type_to_decode() -> Type {
        Evm2AnyTokenTransferAsTuple::abi_type_to_decode()
    }
}

impl TryFrom<Value> for Evm2AnyTokenTransfer {
    type Error = AbiDecodeError;
    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let (source_pool_address, dest_token_address, extra_data, amount, dest_exec_data) =
            Evm2AnyTokenTransferAsTuple::try_from(value)?;
        Ok(Evm2AnyTokenTransfer {
            source_pool_address,
            dest_token_address,
            extra_data,
            amount,
            dest_exec_data,
        })
    }
}

pub struct Any2EvmRampMessage {
    header: RampMessageHeader,
    sender: Bytes,
    data: Bytes,
    receiver: Address,
    gas_limit: U256,
    token_amounts: Vec<Any2EvmTokenTransfer>,
}

#[derive(Debug, Clone)]
struct Any2EvmTokenTransfer {
    source_pool_address: Bytes,
    dest_token_address: Address,
    dest_gas_amount: u32,
    extra_data: Bytes,
    amount: U256,
}

impl From<Any2EvmTokenTransfer> for Value {
    fn from(value: Any2EvmTokenTransfer) -> Self {
        (
            value.source_pool_address,
            value.dest_token_address,
            value.dest_gas_amount,
            value.extra_data,
            value.amount,
        )
            .into()
    }
}

pub struct Any2EvmRampMessageToHash {
    pub any2evm_ramp_message: Any2EvmRampMessage,
    pub onramp_address: Address,
}

impl Any2EvmRampMessageToHash {
    pub fn hash(self) -> Bytes32 {
        fn h(x: impl AsRef<[u8]>) -> Bytes32 {
            Bytes32::from(keccak256(x.as_ref()))
        }

        let domain_separator = h(b"Any2EVMMessageHashV1");
        let metadata_hash = h((
            domain_separator,
            self.any2evm_ramp_message.header.source_chain_selector,
            self.any2evm_ramp_message.header.dest_chain_selector,
            h(self.onramp_address.abi_encode()),
        )
            .abi_encode());
        h((
            Bytes32::from(*afn::merkle::LEAF_DOMAIN_SEPARATOR),
            metadata_hash,
            h((
                self.any2evm_ramp_message.header.message_id,
                self.any2evm_ramp_message.receiver,
                self.any2evm_ramp_message.header.sequence_number,
                self.any2evm_ramp_message.gas_limit,
                self.any2evm_ramp_message.header.nonce,
            )
                .abi_encode()),
            h(self.any2evm_ramp_message.sender),
            h(self.any2evm_ramp_message.data),
            h(self.any2evm_ramp_message.token_amounts.abi_encode()),
        )
            .abi_encode())
    }
}

impl TryFrom<Evm2AnyRampMessage> for Any2EvmRampMessage {
    type Error = anyhow::Error;
    fn try_from(value: Evm2AnyRampMessage) -> Result<Self> {
        Ok(Any2EvmRampMessage {
            header: value.header,
            sender: value.sender.abi_encode(),
            data: value.data,
            receiver: Address::abi_decode(value.receiver).context("invalid receiver")?,
            gas_limit: EvmExtraArgsV2::abi_decode_with_selector(value.extra_args)
                .context("invalid extra_args")?
                .gas_limit,
            token_amounts: value
                .token_amounts
                .into_iter()
                .map(|x| {
                    Ok(Any2EvmTokenTransfer {
                        source_pool_address: x.source_pool_address.abi_encode(),
                        dest_token_address: Address::abi_decode(x.dest_token_address)
                            .context("invalid dest_token_address")?,
                        dest_gas_amount: u32::abi_decode(x.dest_exec_data)
                            .context("invalid dest_gas_amount")?,
                        extra_data: x.extra_data,
                        amount: x.amount,
                    })
                })
                .collect::<Result<Vec<_>>>()
                .context("invalid token_amounts")?,
        })
    }
}
