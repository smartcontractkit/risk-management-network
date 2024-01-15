use crate::types::Value;
use minieth::{
    bytes::{Address, Bytes, Bytes32},
    u256::U256,
};

pub trait AbiEncode: Sized {
    fn abi_encode_into(self, bytes: &mut Vec<u8>);

    fn abi_encode(self) -> Bytes {
        let mut bytes = Vec::new();
        self.abi_encode_into(&mut bytes);
        bytes.into()
    }
}

impl<T> AbiEncode for T
where
    T: Into<Value>,
{
    fn abi_encode_into(self, bytes: &mut Vec<u8>) {
        self.into().encode_into(bytes)
    }
}

macro_rules! value_to_encode_impl_from_prim_type {
    ($type: ident, $constructor: ident) => {
        impl From<$type> for Value {
            fn from(v: $type) -> Self {
                Value::$constructor(v)
            }
        }
    };
}
value_to_encode_impl_from_prim_type!(bool, Bool);
value_to_encode_impl_from_prim_type!(U256, U256);
value_to_encode_impl_from_prim_type!(u128, U128);
value_to_encode_impl_from_prim_type!(u64, U64);
value_to_encode_impl_from_prim_type!(u32, U32);
value_to_encode_impl_from_prim_type!(u16, U16);
value_to_encode_impl_from_prim_type!(u8, U8);
value_to_encode_impl_from_prim_type!(Address, Address);
value_to_encode_impl_from_prim_type!(Bytes32, Bytes32);
value_to_encode_impl_from_prim_type!(Bytes, Bytes);
value_to_encode_impl_from_prim_type!(String, String);

impl<T> From<Vec<T>> for Value
where
    T: Into<Value> + Clone,
{
    fn from(value: Vec<T>) -> Self {
        Value::DynamicArray(value.into_iter().map(T::into).collect())
    }
}

macro_rules! impl_abi_encode_tuple {
    ($( ($type: ident , $var: ident), )+) => {
        impl<$($type:Into<Value> , )+> From<($($type, )+)> for Value {
            fn from(($($var, )+): ($($type, )+)) -> Self {
                Value::Tuple(vec![$($var.into() , )+])
            }
        }
    };
}
impl From<()> for Value {
    fn from(_v: ()) -> Self {
        Value::Tuple(vec![])
    }
}
impl_abi_encode_tuple!((A, a),);
impl_abi_encode_tuple!((A, a), (B, b),);
impl_abi_encode_tuple!((A, a), (B, b), (C, c),);
impl_abi_encode_tuple!((A, a), (B, b), (C, c), (D, d),);
impl_abi_encode_tuple!((A, a), (B, b), (C, c), (D, d), (E, e),);
impl_abi_encode_tuple!((A, a), (B, b), (C, c), (D, d), (E, e), (F, f),);
impl_abi_encode_tuple!(
    (A, a),
    (B, b),
    (C, c),
    (D, d),
    (E, e),
    (F, f),
    (G, g),
    (H, h),
);

impl_abi_encode_tuple!(
    (A, a),
    (B, b),
    (C, c),
    (D, d),
    (E, e),
    (F, f),
    (G, g),
    (H, h),
    (I, i),
    (J, j),
    (K, k),
    (L, l),
);
