use crate::chain_selector::ChainSelector;
use miniabi::abi_encode::AbiEncode;
use minieth::{
    bytes::{Address, Bytes32},
    keccak::{keccak256, Hasher, Keccak},
    u256::U256,
};

#[derive(Debug, Clone)]
pub struct MessageMetadata {
    pub onramp_address: Address,
    pub source_chain_selector: ChainSelector,
    pub dest_chain_selector: ChainSelector,
}

pub fn metadata_hash(message_type: &str, metadata: &MessageMetadata) -> Bytes32 {
    let mut output = [0u8; 32];
    let mut hasher = Keccak::v256();
    hasher.update(&keccak256(message_type.as_ref()));
    hasher.update(metadata.source_chain_selector.abi_encode().as_ref());
    hasher.update(metadata.dest_chain_selector.abi_encode().as_ref());
    hasher.update(metadata.onramp_address.abi_encode().as_ref());
    hasher.finalize(&mut output);
    Bytes32::from(output)
}

pub fn hash_abi_encodables<A: AbiEncode + Copy>(xs: &[A]) -> Bytes32 {
    let mut output = [0u8; 32];
    let mut hasher = Keccak::v256();
    hasher.update(U256::from(32u64).abi_encode().as_ref());
    hasher.update(U256::from(xs.len() as u128).abi_encode().as_ref());
    for x in xs {
        hasher.update(x.abi_encode().as_ref())
    }
    hasher.finalize(&mut output);
    Bytes32::from(output)
}

pub trait Hashable {
    fn hash(&self, metadata: &MessageMetadata) -> Bytes32;
}
