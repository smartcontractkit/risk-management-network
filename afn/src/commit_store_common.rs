use crate::merkle::root_from_hashed_leafs;
use crate::onramp_traits::OnRampReader;
use anyhow::{anyhow, Result};
use miniabi::{
    abi_decode::{AbiDecodeError, AbiTypeToDecode},
    types::{Type, Value},
};
use minieth::{
    bytes::{Address, Bytes32},
    u256::U256,
};

#[derive(Debug, Eq, Hash, PartialEq, Clone, Copy)]
pub struct Interval {
    pub min: u64,
    pub max: u64,
}
type IntervalAsTuple = (u64, u64);
impl AbiTypeToDecode for Interval {
    fn abi_type_to_decode() -> Type {
        IntervalAsTuple::abi_type_to_decode()
    }
}

impl TryFrom<Value> for Interval {
    type Error = AbiDecodeError;
    fn try_from(v: Value) -> Result<Self, Self::Error> {
        let (min, max) = IntervalAsTuple::try_from(v)?;
        Ok(Self { min, max })
    }
}

#[derive(Debug, Eq, Hash, PartialEq, Clone)]
pub struct CommitReport<PriceUpdates: AbiTypeToDecode> {
    pub price_updates: PriceUpdates,
    pub interval: Interval,
    pub merkle_root: Bytes32,
}

impl<T: AbiTypeToDecode> From<CommitReport<T>> for RootWithInterval {
    fn from(commit_report: CommitReport<T>) -> Self {
        RootWithInterval {
            root: commit_report.merkle_root,
            interval: commit_report.interval,
        }
    }
}

type CommitReportAsTuple<PriceUpdates> = (PriceUpdates, Interval, Bytes32);

impl<PriceUpdates: AbiTypeToDecode> AbiTypeToDecode for CommitReport<PriceUpdates> {
    fn abi_type_to_decode() -> Type {
        CommitReportAsTuple::<PriceUpdates>::abi_type_to_decode()
    }
}
impl<PriceUpdates: AbiTypeToDecode> TryFrom<Value> for CommitReport<PriceUpdates>
where
    CommitReportAsTuple<PriceUpdates>: TryFrom<Value, Error = AbiDecodeError>,
{
    type Error = AbiDecodeError;
    fn try_from(v: Value) -> Result<Self, Self::Error> {
        let (price_updates, interval, merkle_root) =
            CommitReportAsTuple::<PriceUpdates>::try_from(v)?;
        Ok(Self {
            price_updates,
            interval,
            merkle_root,
        })
    }
}

#[derive(Debug, Eq, Hash, PartialEq, Clone)]
pub struct TokenPriceUpdate {
    pub source_token: Address,
    pub usd_per_token: U256,
}
type TokenPriceUpdateAsTuple = (Address, U256);
impl AbiTypeToDecode for TokenPriceUpdate {
    fn abi_type_to_decode() -> Type {
        TokenPriceUpdateAsTuple::abi_type_to_decode()
    }
}
impl TryFrom<Value> for TokenPriceUpdate {
    type Error = AbiDecodeError;
    fn try_from(v: Value) -> Result<Self, Self::Error> {
        let (source_token, usd_per_token) = TokenPriceUpdateAsTuple::try_from(v)?;
        Ok(Self {
            source_token,
            usd_per_token,
        })
    }
}

#[derive(Debug, Eq, Hash, PartialEq, Clone)]
pub struct GasPriceUpdate {
    pub dest_chain_selector: u64,
    pub usd_per_unit_gas: U256,
}

type GasPriceUpdateAsTuple = (u64, U256);
impl AbiTypeToDecode for GasPriceUpdate {
    fn abi_type_to_decode() -> Type {
        GasPriceUpdateAsTuple::abi_type_to_decode()
    }
}
impl TryFrom<Value> for GasPriceUpdate {
    type Error = AbiDecodeError;
    fn try_from(v: Value) -> Result<Self, Self::Error> {
        let (dest_chain_selector, usd_per_unit_gas) = GasPriceUpdateAsTuple::try_from(v)?;
        Ok(Self {
            dest_chain_selector,
            usd_per_unit_gas,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RootWithInterval {
    pub root: Bytes32,
    pub interval: Interval,
}

pub fn verify_root_with_interval(
    ri: &RootWithInterval,
    reader: &impl OnRampReader,
) -> Result<bool> {
    Ok(ri.root == compute_merkle_root(&ri.interval, reader)?)
}

fn is_valid_interval(interval: &Interval) -> bool {
    interval.max >= interval.min
}

fn compute_merkle_root(interval: &Interval, reader: &impl OnRampReader) -> Result<Bytes32> {
    if !is_valid_interval(interval) {
        return Err(anyhow!(
            "compute_merkle_root: invalid interval {:?}",
            interval
        ));
    }
    let leaves: Vec<Bytes32> = reader.get_message_hashes(interval).ok_or_else(|| {
        anyhow!(
            "compute_merkle_root: failed to get message hashes for interval: {:?}",
            interval
        )
    })?;
    let expected_num_leaves = interval.max - interval.min + 1;
    if leaves.len() as u64 != expected_num_leaves {
        return Err(anyhow!(
                "compute_merkle_root: incorrect number of messages in the interval {:?}, expected {}, got {}.",
                interval,
                expected_num_leaves,
                leaves.len(),
            ));
    }
    let root = root_from_hashed_leafs(&leaves).ok_or_else(|| {
        anyhow!(
            "compute_merkle_root: failed to compute root from leaves {:?}",
            leaves
        )
    })?;
    Ok(root)
}
