use minieth::{
    bytes::{Address, Bytes, Bytes32},
    u256::U256,
};

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub enum Type {
    Bool,
    U256,
    U128,
    U64,
    U32,
    U16,
    U8,
    Address,
    Bytes32,
    Bytes,
    String,
    DynamicArray(Box<Type>),
    Tuple(Vec<Type>),
}

impl Type {
    pub(crate) fn all_static(tuple: &[Type]) -> bool {
        tuple.iter().all(Type::is_static)
    }
    pub(crate) fn is_static(&self) -> bool {
        match self {
            Type::Bool
            | Type::U256
            | Type::U128
            | Type::U64
            | Type::U32
            | Type::U16
            | Type::U8
            | Type::Address
            | Type::Bytes32 => true,
            Type::Bytes | Type::String | Type::DynamicArray(_) => false,
            Type::Tuple(tuple) => Self::all_static(tuple),
        }
    }
}

#[derive(PartialEq, Eq, Debug, Clone)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub enum Value {
    Bool(bool),
    U256(U256),
    U128(u128),
    U64(u64),
    U32(u32),
    U16(u16),
    U8(u8),
    Address(Address),
    Bytes32(Bytes32),
    Bytes(Bytes),
    String(String),
    Tuple(Vec<Value>),
    DynamicArray(Vec<Value>),
}
impl Value {
    pub(crate) fn all_static(tuple: &[Value]) -> bool {
        tuple.iter().all(Value::is_static)
    }
    pub(crate) fn is_static(&self) -> bool {
        match self {
            Self::Bool(_)
            | Self::U256(_)
            | Self::U128(_)
            | Self::U64(_)
            | Self::U32(_)
            | Self::U16(_)
            | Self::U8(_)
            | Self::Address(_)
            | Self::Bytes32(_) => true,
            Self::Bytes(_) | Self::String(_) | Self::DynamicArray(_) => false,
            Self::Tuple(tuple) => Self::all_static(tuple),
        }
    }
}
