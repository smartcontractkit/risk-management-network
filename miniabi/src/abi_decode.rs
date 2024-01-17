use crate::types::{Type, Value};
use minieth::{
    bytes::{Address, Bytes, Bytes32},
    u256::U256,
};
use std::convert::TryFrom;
use thiserror::Error;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum AbiDecodeError {
    #[error("invalid format")]
    InvalidFormat,
    #[error("invalid encoding size")]
    InvalidEncodingSize,
    #[error("invalid bool encoding")]
    InvalidBoolEncoding,
    #[error("invalid uint encoding")]
    InvalidUintEncoding,
    #[error("invalid usize encoding")]
    InvalidUsizeEncoding,
    #[error("invalid address encoding")]
    InvalidAddressEncoding,
    #[error("invalid bytes encoding")]
    InvalidBytesEncoding,
    #[error("invalid string encoding")]
    InvalidStringEncoding,
    #[error("invalid decoded value error")]
    InvalidValueError,
    #[error("zero-sized type not supported")]
    ZeroSizedTypeError,
}

pub trait AbiTypeToDecode {
    fn abi_type_to_decode() -> Type;
}

pub trait AbiDecode: AbiTypeToDecode + Sized {
    fn abi_decode(bytes: impl AsRef<[u8]>) -> Result<Self, AbiDecodeError>;
}

impl<T> AbiDecode for T
where
    T: AbiTypeToDecode + TryFrom<Value, Error = AbiDecodeError>,
{
    fn abi_decode(bytes: impl AsRef<[u8]>) -> Result<Self, AbiDecodeError> {
        let v = T::abi_type_to_decode().decode(bytes.as_ref())?;
        T::try_from(v)
    }
}

macro_rules! impl_into_type_to_decode_prim_type {
    ($type: ident, $constructor: ident) => {
        impl AbiTypeToDecode for $type {
            fn abi_type_to_decode() -> Type {
                Type::$constructor
            }
        }
    };
}
impl_into_type_to_decode_prim_type!(bool, Bool);
impl_into_type_to_decode_prim_type!(U256, U256);
impl_into_type_to_decode_prim_type!(u128, U128);
impl_into_type_to_decode_prim_type!(u64, U64);
impl_into_type_to_decode_prim_type!(u32, U32);
impl_into_type_to_decode_prim_type!(u16, U16);
impl_into_type_to_decode_prim_type!(u8, U8);
impl_into_type_to_decode_prim_type!(Address, Address);
impl_into_type_to_decode_prim_type!(Bytes32, Bytes32);
impl_into_type_to_decode_prim_type!(Bytes, Bytes);
impl_into_type_to_decode_prim_type!(String, String);

impl<T: AbiTypeToDecode> AbiTypeToDecode for Vec<T> {
    fn abi_type_to_decode() -> Type {
        Type::DynamicArray(Box::new(T::abi_type_to_decode()))
    }
}

macro_rules! impl_into_type_to_decode_tuple {
    ($($type: ident, )+) => {
        impl<$($type:AbiTypeToDecode, )+> AbiTypeToDecode for ($($type, )+) {
            fn abi_type_to_decode() -> Type {
                Type::Tuple(vec![
                    $($type::abi_type_to_decode(), )+
                ])
            }
        }
    };
}
impl_into_type_to_decode_tuple!(A,);
impl_into_type_to_decode_tuple!(A, B,);
impl_into_type_to_decode_tuple!(A, B, C,);
impl_into_type_to_decode_tuple!(A, B, C, D,);
impl_into_type_to_decode_tuple!(A, B, C, D, E,);
impl_into_type_to_decode_tuple!(A, B, C, D, E, F,);
impl_into_type_to_decode_tuple!(A, B, C, D, E, F, G,);
impl_into_type_to_decode_tuple!(A, B, C, D, E, F, G, H, I,);
impl_into_type_to_decode_tuple!(A, B, C, D, E, F, G, H, I, J, K, L,);
impl_into_type_to_decode_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M,);

macro_rules! impl_try_from_decoded_value_prim_type {
    ($type: ident, $constructor: ident) => {
        impl TryFrom<Value> for $type {
            type Error = AbiDecodeError;
            fn try_from(v: Value) -> Result<Self, Self::Error> {
                if let Value::$constructor(v) = v {
                    Ok(v.clone())
                } else {
                    Err(AbiDecodeError::InvalidValueError)
                }
            }
        }
    };
}

impl_try_from_decoded_value_prim_type!(bool, Bool);
impl_try_from_decoded_value_prim_type!(U256, U256);
impl_try_from_decoded_value_prim_type!(u128, U128);
impl_try_from_decoded_value_prim_type!(u64, U64);
impl_try_from_decoded_value_prim_type!(u32, U32);
impl_try_from_decoded_value_prim_type!(u16, U16);
impl_try_from_decoded_value_prim_type!(u8, U8);
impl_try_from_decoded_value_prim_type!(Address, Address);
impl_try_from_decoded_value_prim_type!(Bytes32, Bytes32);
impl_try_from_decoded_value_prim_type!(Bytes, Bytes);
impl_try_from_decoded_value_prim_type!(String, String);

impl<T> TryFrom<Value> for Vec<T>
where
    T: TryFrom<Value, Error = AbiDecodeError>,
{
    type Error = AbiDecodeError;
    fn try_from(v: Value) -> Result<Self, Self::Error> {
        if let Value::DynamicArray(v) = v {
            v.into_iter().map(|v| T::try_from(v)).collect()
        } else {
            Err(AbiDecodeError::InvalidValueError)
        }
    }
}

macro_rules! impl_try_from_decoded_value_tuple {
    ($num: expr, $( ($type: ident , $idx: expr), )+) => {
        impl<$($type:TryFrom<Value, Error = AbiDecodeError>, )+> TryFrom<Value> for ($($type, )+) {
            type Error = AbiDecodeError;
            fn try_from(v: Value) -> Result<Self, Self::Error> {
                if let Value::Tuple(vec) = v {
                    if vec.len() == $num {
                        Ok((
                            $($type::try_from(vec[$idx].clone())?, )+
                        ))
                    } else {
                        Err(AbiDecodeError::InvalidValueError)
                    }
                } else {
                    Err(AbiDecodeError::InvalidValueError)
                }
            }
        }
    };
}
impl_try_from_decoded_value_tuple!(1, (A, 0),);
impl_try_from_decoded_value_tuple!(2, (A, 0), (B, 1),);
impl_try_from_decoded_value_tuple!(3, (A, 0), (B, 1), (C, 2),);
impl_try_from_decoded_value_tuple!(4, (A, 0), (B, 1), (C, 2), (D, 3),);
impl_try_from_decoded_value_tuple!(5, (A, 0), (B, 1), (C, 2), (D, 3), (E, 4),);
impl_try_from_decoded_value_tuple!(6, (A, 0), (B, 1), (C, 2), (D, 3), (E, 4), (F, 5),);
impl_try_from_decoded_value_tuple!(7, (A, 0), (B, 1), (C, 2), (D, 3), (E, 4), (F, 5), (G, 6),);
impl_try_from_decoded_value_tuple!(
    9,
    (A, 0),
    (B, 1),
    (C, 2),
    (D, 3),
    (E, 4),
    (F, 5),
    (G, 6),
    (H, 7),
    (I, 8),
);
impl_try_from_decoded_value_tuple!(
    12,
    (A, 0),
    (B, 1),
    (C, 2),
    (D, 3),
    (E, 4),
    (F, 5),
    (G, 6),
    (H, 7),
    (I, 8),
    (J, 9),
    (K, 10),
    (L, 11),
);
impl_try_from_decoded_value_tuple!(
    13,
    (A, 0),
    (B, 1),
    (C, 2),
    (D, 3),
    (E, 4),
    (F, 5),
    (G, 6),
    (H, 7),
    (I, 8),
    (J, 9),
    (K, 10),
    (L, 11),
    (M, 12),
);
