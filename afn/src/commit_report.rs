use crate::commit_store::{Interval, PriceUpdates};
use crate::merkle::root_from_hashed_leafs;
use crate::onramp_traits::OnRampReader;
use anyhow::{anyhow, Result};
use minieth::bytes::Bytes32;

#[derive(Debug, Clone, Copy)]
pub struct RootWithInterval {
    pub root: Bytes32,
    pub interval: crate::commit_store::Interval,
}

#[derive(Debug, Eq, Hash, PartialEq, Clone)]
pub struct CommitReport {
    pub price_updates: PriceUpdates,
    pub interval: Interval,
    pub merkle_root: Bytes32,
}

impl From<CommitReport> for RootWithInterval {
    fn from(commit_report: CommitReport) -> Self {
        RootWithInterval {
            root: commit_report.merkle_root,
            interval: commit_report.interval,
        }
    }
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
