use minieth::{
    bytes::{Address, Bytes, Bytes32},
    u256::U256,
};
use std::convert::TryFrom;
use thiserror::Error;

#[derive(Debug, Clone)]
pub enum TypeToDecode {
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
    DynamicArray(Box<TypeToDecode>),
    Tuple(Vec<TypeToDecode>),
}

#[derive(PartialEq, Eq, Debug, Clone)]
pub enum DecodedValue {
    Bool(bool),
    U256(U256),
    U128(u128),
    U64(u64),
    U32(u32),
    U16(u16),
    U8(u8),
    Address(Address),
    Bytes(Bytes),
    Bytes32(Bytes32),
    Tuple(Vec<DecodedValue>),
    Array(Vec<DecodedValue>),
}

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
    #[error("invalid decoded value error")]
    InvalidDecodedValueError,
}

pub trait AbiTypeToDecode {
    fn abi_type_to_decode() -> TypeToDecode;
}

pub trait AbiDecode: AbiTypeToDecode + Sized {
    fn abi_decode(bytes: impl AsRef<[u8]>) -> Result<Self, AbiDecodeError>;
}

impl<T> AbiDecode for T
where
    T: AbiTypeToDecode + TryFrom<DecodedValue, Error = AbiDecodeError>,
{
    fn abi_decode(bytes: impl AsRef<[u8]>) -> Result<Self, AbiDecodeError> {
        let v = T::abi_type_to_decode().decode(bytes.as_ref())?;
        T::try_from(v)
    }
}

macro_rules! impl_into_type_to_decode_prim_type {
    ($type: ident, $constructor: ident) => {
        impl AbiTypeToDecode for $type {
            fn abi_type_to_decode() -> TypeToDecode {
                TypeToDecode::$constructor
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

impl<T: AbiTypeToDecode> AbiTypeToDecode for Vec<T> {
    fn abi_type_to_decode() -> TypeToDecode {
        TypeToDecode::DynamicArray(Box::new(T::abi_type_to_decode()))
    }
}
impl AbiTypeToDecode for Bytes {
    fn abi_type_to_decode() -> TypeToDecode {
        TypeToDecode::Bytes
    }
}

macro_rules! impl_into_type_to_decode_tuple {
    ($($type: ident, )+) => {
        impl<$($type:AbiTypeToDecode, )+> AbiTypeToDecode for ($($type, )+) {
            fn abi_type_to_decode() -> TypeToDecode {
                TypeToDecode::Tuple(vec![
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

macro_rules! impl_try_from_decoded_value_prim_type {
    ($type: ident, $constructor: ident) => {
        impl TryFrom<DecodedValue> for $type {
            type Error = AbiDecodeError;
            fn try_from(v: DecodedValue) -> Result<Self, Self::Error> {
                if let DecodedValue::$constructor(v) = v {
                    Ok(v.clone())
                } else {
                    Err(AbiDecodeError::InvalidDecodedValueError)
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

impl<T> TryFrom<DecodedValue> for Vec<T>
where
    T: TryFrom<DecodedValue, Error = AbiDecodeError>,
{
    type Error = AbiDecodeError;
    fn try_from(v: DecodedValue) -> Result<Self, Self::Error> {
        if let DecodedValue::Array(v) = v {
            v.into_iter().map(|v| T::try_from(v)).collect()
        } else {
            Err(AbiDecodeError::InvalidDecodedValueError)
        }
    }
}

macro_rules! impl_try_from_decoded_value_tuple {
    ($num: expr, $( ($type: ident , $idx: expr), )+) => {
        impl<$($type:TryFrom<DecodedValue, Error = AbiDecodeError>, )+> TryFrom<DecodedValue> for ($($type, )+) {
            type Error = AbiDecodeError;
            fn try_from(v: DecodedValue) -> Result<Self, Self::Error> {
                if let DecodedValue::Tuple(vec) = v {
                    if vec.len() == $num {
                        Ok((
                            $($type::try_from(vec[$idx].clone())?, )+
                        ))
                    } else {
                        Err(AbiDecodeError::InvalidDecodedValueError)
                    }
                } else {
                    Err(AbiDecodeError::InvalidDecodedValueError)
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
