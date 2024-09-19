use crate::chain_selector::ChainSelector;
use miniabi::{
    abi_decode::{AbiDecodeError, AbiTypeToDecode},
    types::{Type, Value},
};
use minieth::bytes::Address;

#[derive(Debug, PartialEq, Eq)]
pub struct StaticConfigV1_5 {
    pub chain_selector: ChainSelector,
    pub src_chain_selector: ChainSelector,
    pub commit_store: Address,
    pub on_ramp: Address,
}

type StaticConfigV1_5AsTuple = (Address, u64, u64, Address, Address, Address, Address);

impl AbiTypeToDecode for StaticConfigV1_5 {
    fn abi_type_to_decode() -> Type {
        StaticConfigV1_5AsTuple::abi_type_to_decode()
    }
}

impl TryFrom<Value> for StaticConfigV1_5 {
    type Error = AbiDecodeError;
    fn try_from(v: Value) -> Result<Self, Self::Error> {
        let (
            commit_store,
            chain_selector,
            src_chain_selector,
            on_ramp,
            _prev_off_ramp,
            _arm_proxy,
            _token_admin_registry,
        ) = StaticConfigV1_5AsTuple::try_from(v)?;
        Ok(Self {
            chain_selector,
            src_chain_selector,
            commit_store,
            on_ramp,
        })
    }
}
