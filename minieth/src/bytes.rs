use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::ops::{Deref, Index, RangeFrom, RangeFull};
use std::str::FromStr;
use thiserror::Error;

use crate::hex;
use crate::rlp::{rlp_encode_bytes, RlpEncodable};

#[derive(Error, Debug)]
pub enum BytesDecodeError {
    #[error("invalid size")]
    InvalidSize,
    #[error("non-hex character encountered")]
    InvalidHex,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SizedBytes<const N: usize>([u8; N]);

impl<const N: usize> From<[u8; N]> for SizedBytes<N> {
    fn from(b: [u8; N]) -> Self {
        Self(b)
    }
}

impl<const N: usize> From<SizedBytes<N>> for [u8; N] {
    fn from(b: SizedBytes<N>) -> [u8; N] {
        b.0
    }
}

impl<const N: usize> FromStr for SizedBytes<N> {
    type Err = BytesDecodeError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let arr = hex::decode(s).ok_or(Self::Err::InvalidHex)?;
        let sized_arr: [u8; N] = arr
            .as_slice()
            .try_into()
            .map_err(|_| Self::Err::InvalidSize)?;
        Ok(sized_arr.into())
    }
}

impl<'de, const N: usize> Deserialize<'de> for SizedBytes<N> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if deserializer.is_human_readable() {
            let s = String::deserialize(deserializer)?;
            FromStr::from_str(&s).map_err(de::Error::custom)
        } else {
            let bytes = <&[u8]>::deserialize(deserializer)?;
            Ok(Self::from(
                <[u8; N]>::try_from(bytes).map_err(de::Error::custom)?,
            ))
        }
    }
}

impl<const N: usize> Serialize for SizedBytes<N> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            serializer.serialize_str(&self.to_string())
        } else {
            serializer.serialize_bytes(self.as_slice())
        }
    }
}

impl<const N: usize> SizedBytes<N> {
    pub fn as_slice(&self) -> &[u8] {
        self.0.as_slice()
    }

    pub fn from_vec(v: Vec<u8>) -> Result<Self, <[u8; N] as TryFrom<Vec<u8>>>::Error> {
        let array: [u8; N] = v.try_into()?;
        Ok(array.into())
    }
}

impl<const N: usize> AsRef<[u8]> for SizedBytes<N> {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl<const N: usize> Deref for SizedBytes<N> {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<const N: usize> Index<RangeFull> for SizedBytes<N> {
    type Output = [u8];
    fn index(&self, idx: RangeFull) -> &Self::Output {
        self.0.index(idx)
    }
}

impl<const N: usize> RlpEncodable for SizedBytes<N> {
    fn to_rlp_bytes(&self) -> Vec<u8> {
        rlp_encode_bytes(&self[..])
    }
}

impl<const N: usize> fmt::Display for SizedBytes<N> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", hex::encode(self.as_slice()))
    }
}

impl<const N: usize> fmt::Debug for SizedBytes<N> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Bytes{}({})", N, self)
    }
}

impl<const N: usize> Default for SizedBytes<N> {
    fn default() -> Self {
        [0u8; N].into()
    }
}

#[cfg(feature = "quickcheck")]
impl<const N: usize> quickcheck::Arbitrary for SizedBytes<N> {
    fn arbitrary(g: &mut quickcheck::Gen) -> Self {
        let mut r = [0u8; N];
        for v in r.iter_mut() {
            *v = u8::arbitrary(g);
        }
        r.into()
    }
}

pub type Bytes20 = SizedBytes<20>;
pub type Bytes32 = SizedBytes<32>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bytes(Vec<u8>);

impl From<Bytes> for Vec<u8> {
    fn from(b: Bytes) -> Self {
        b.0
    }
}

impl From<Vec<u8>> for Bytes {
    fn from(v: Vec<u8>) -> Self {
        Self(v)
    }
}

impl FromStr for Bytes {
    type Err = BytesDecodeError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(hex::decode(s).ok_or(Self::Err::InvalidHex)?.into())
    }
}

impl AsRef<[u8]> for Bytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Deref for Bytes {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Bytes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if deserializer.is_human_readable() {
            let s = String::deserialize(deserializer)?;
            FromStr::from_str(&s).map_err(de::Error::custom)
        } else {
            let bytes = <&[u8]>::deserialize(deserializer)?;
            Ok(bytes.to_vec().into())
        }
    }
}

impl Serialize for Bytes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            serializer.serialize_str(&self.to_string())
        } else {
            serializer.serialize_bytes(self.0.as_slice())
        }
    }
}

impl RlpEncodable for Bytes {
    fn to_rlp_bytes(&self) -> Vec<u8> {
        rlp_encode_bytes(&self.0)
    }
}

impl ToString for Bytes {
    fn to_string(&self) -> String {
        hex::encode(self.as_ref())
    }
}

impl Index<RangeFrom<usize>> for Bytes {
    type Output = [u8];
    fn index(&self, index: RangeFrom<usize>) -> &Self::Output {
        self.0.index(index)
    }
}

impl Bytes {
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn from_ref(r: impl AsRef<[u8]>) -> Self {
        Self(r.as_ref().into())
    }
}

pub type Address = Bytes20;

pub fn prepend(prefix: u8, bytes: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(bytes.len() + 1);
    v.push(prefix);
    v.extend(bytes);
    v
}
