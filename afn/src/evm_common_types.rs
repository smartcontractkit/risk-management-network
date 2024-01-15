use miniabi::abi_decode::{AbiDecodeError, AbiTypeToDecode};
use miniabi::types::{Type, Value};
use minieth::bytes::Address;
use minieth::u256::U256;

#[derive(Clone, Debug, Copy, PartialEq)]
pub struct EVMTokenAmount {
    pub token: Address,
    pub amount: U256,
}
type EVMTokenAmountAsTuple = (Address, U256);

impl From<EVMTokenAmount> for Value {
    fn from(v: EVMTokenAmount) -> Self {
        (v.token, v.amount).into()
    }
}

impl AbiTypeToDecode for EVMTokenAmount {
    fn abi_type_to_decode() -> Type {
        EVMTokenAmountAsTuple::abi_type_to_decode()
    }
}

impl TryFrom<Value> for EVMTokenAmount {
    type Error = AbiDecodeError;
    fn try_from(v: Value) -> Result<Self, Self::Error> {
        let (token, amount) = EVMTokenAmountAsTuple::try_from(v)?;
        Ok(Self { token, amount })
    }
}
