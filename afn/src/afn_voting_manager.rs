use crate::{
    common::{ChainName, LaneId},
    config::ChainConfig,
    curse_beacon::CurseBeacon,
    key_types::BlessCurseKeys,
    worker::ShutdownHandleGroup,
    {
        lane_bless_status::LaneBlessStatusWorker, vote_to_bless_worker::VoteToBlessWorker,
        vote_to_curse_worker::VoteToCurseWorker,
    },
};
use anyhow::Result;
use minieth::rpc::Rpc;
use std::{collections::HashMap, str::FromStr, sync::Arc};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum VotingMode {
    Active,
    DryRun,
    Passive,
}

impl FromStr for VotingMode {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "active" => Ok(Self::Active),
            "dryrun" => Ok(Self::DryRun),
            "passive" => Ok(Self::Passive),
            _ => Err(format!("unknown voting mode {:?}", s)),
        }
    }
}

pub struct AFNVotingManager {
    pub chain_name: ChainName,
    pub vote_to_bless_worker: VoteToBlessWorker,
    pub vote_to_curse_worker: VoteToCurseWorker,
}

impl AFNVotingManager {
    pub fn new_and_spawn_workers(
        ctx: &Arc<crate::worker::Context>,
        rpc: Arc<Rpc>,
        config: &ChainConfig,
        onchain_config: crate::afn_contract::OnchainConfig,
        keys: &BlessCurseKeys,
        lane_bless_status_workers: HashMap<LaneId, Arc<LaneBlessStatusWorker>>,
        curse_beacon: Arc<CurseBeacon>,
        mode: VotingMode,
    ) -> Result<(Self, ShutdownHandleGroup)> {
        let mut shutdown_handles = ShutdownHandleGroup::default();
        let vote_to_bless_worker = shutdown_handles.add(VoteToBlessWorker::spawn(
            ctx,
            Arc::clone(&rpc),
            config,
            onchain_config,
            lane_bless_status_workers,
            crate::config::VOTE_TO_BLESS_WORKER_POLL_INTERVAL,
            keys.bless,
            Arc::clone(&curse_beacon),
            mode,
        )?);
        let vote_to_curse_worker = shutdown_handles.add(VoteToCurseWorker::spawn(
            ctx,
            Arc::clone(&rpc),
            config,
            crate::config::VOTE_TO_CURSE_WORKER_POLL_INTERVAL,
            keys.curse,
            Arc::clone(&curse_beacon),
            mode,
        )?);
        Ok((
            Self {
                chain_name: config.name,
                vote_to_bless_worker,
                vote_to_curse_worker,
            },
            shutdown_handles,
        ))
    }
}
