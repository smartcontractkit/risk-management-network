use crate::bytes::{Bytes, BytesDecodeError};
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

use crate::rlp::{rlp_encode_numerical, RlpEncodable};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct U256 {
    hi: u128,
    lo: u128,
}

impl U256 {
    pub const MAX: U256 = U256 {
        hi: u128::MAX,
        lo: u128::MAX,
    };
    pub const MIN: U256 = U256 { hi: 0, lo: 0 };
    pub const BITS: u32 = 256u32;

    pub const fn new(value: u128) -> Self {
        Self { hi: 0, lo: value }
    }

    pub fn from_be_bytes(bytes: [u8; 32]) -> Self {
        let hi = u128::from_be_bytes(bytes[..16].try_into().unwrap());
        let lo = u128::from_be_bytes(bytes[16..].try_into().unwrap());
        Self { hi, lo }
    }

    pub fn to_be_bytes(&self) -> [u8; 32] {
        let mut buf = [0; 32];
        buf[0..16].copy_from_slice(&self.hi.to_be_bytes());
        buf[16..32].copy_from_slice(&self.lo.to_be_bytes());
        buf
    }
}

impl fmt::Display for U256 {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.hi == 0 {
            write!(f, "0x{:x}", self.lo)
        } else {
            write!(f, "0x{:x}{:x}", self.hi, self.lo)
        }
    }
}

impl fmt::Debug for U256 {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "U256({self})")
    }
}

impl RlpEncodable for U256 {
    fn to_rlp_bytes(&self) -> Vec<u8> {
        rlp_encode_numerical(&self.to_be_bytes())
    }
}

impl FromStr for U256 {
    type Err = BytesDecodeError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = Bytes::from_str(s)?;
        if bytes.len() > 32 {
            return Err(Self::Err::InvalidSize);
        }
        let mut buf = [0u8; 32];
        buf[32 - bytes.len()..].copy_from_slice(bytes.as_ref());
        Ok(Self::from_be_bytes(buf))
    }
}

impl<T: Into<u128>> From<T> for U256 {
    fn from(value: T) -> Self {
        Self::new(value.into())
    }
}

#[derive(Error, Debug)]
pub enum TryFromU256Error {
    #[error("overflow")]
    Overflow,
}

macro_rules! impl_try_from_u256 {
    ($typ: ident) => {
        impl TryFrom<U256> for $typ {
            type Error = TryFromU256Error;
            fn try_from(value: U256) -> Result<Self, Self::Error> {
                if value.hi != 0 {
                    return Err(Self::Error::Overflow);
                }
                value.lo.try_into().map_err(|_| Self::Error::Overflow)
            }
        }
    };
}
impl_try_from_u256!(u128);
impl_try_from_u256!(u64);
impl_try_from_u256!(u32);
impl_try_from_u256!(u16);
impl_try_from_u256!(u8);

impl<'de> Deserialize<'de> for U256 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        FromStr::from_str(&s).map_err(de::Error::custom)
    }
}

impl Serialize for U256 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}
