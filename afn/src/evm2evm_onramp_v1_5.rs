use miniabi::{
    abi_decode::{AbiDecodeError, AbiTypeToDecode},
    types::{Type, Value},
};
use minieth::bytes::Address;

pub struct StaticConfigV1_5 {
    pub chain_selector: u64,
    pub dest_chain_selector: u64,
}
type StaticConfigV1_5AsTuple = (Address, u64, u64, u64, u128, Address, Address, Address);

impl AbiTypeToDecode for StaticConfigV1_5 {
    fn abi_type_to_decode() -> Type {
        StaticConfigV1_5AsTuple::abi_type_to_decode()
    }
}
impl TryFrom<Value> for StaticConfigV1_5 {
    type Error = AbiDecodeError;
    fn try_from(v: Value) -> Result<Self, Self::Error> {
        let (
            _link_token,
            chain_selector,
            dest_chain_selector,
            _default_tx_gas_limit,
            _max_nop_fees_juels,
            _prev_on_ramp,
            _arm_proxy,
            _token_admin_registry,
        ) = StaticConfigV1_5AsTuple::try_from(v)?;
        Ok(Self {
            chain_selector,
            dest_chain_selector,
        })
    }
}
