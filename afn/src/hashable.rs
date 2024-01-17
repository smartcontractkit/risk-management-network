use crate::chain_selector::ChainSelector;
use miniabi::abi_encode::AbiEncode;
use minieth::{
    bytes::{Address, Bytes32},
    keccak::keccak256,
};

#[derive(Debug, Clone)]
pub struct MessageMetadata {
    pub onramp_address: Address,
    pub source_chain_selector: ChainSelector,
    pub dest_chain_selector: ChainSelector,
}

pub fn metadata_hash(message_type: &str, metadata: &MessageMetadata) -> Bytes32 {
    Bytes32::from(keccak256(
        (
            Bytes32::from(keccak256(message_type.as_ref())),
            metadata.source_chain_selector,
            metadata.dest_chain_selector,
            metadata.onramp_address,
        )
            .abi_encode()
            .as_ref(),
    ))
}

pub trait Hashable {
    fn hash(&self, metadata: &MessageMetadata) -> Bytes32;
}
