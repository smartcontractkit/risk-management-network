use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, RwLock},
};

use minieth::bytes::Bytes16;

use crate::{
    afn_contract::AFNType,
    afn_contract_v1_5::CurseSubjectV1_5,
    chain_status::ChainStatus,
    chain_status_worker::{ChainStatusProvider, ChainStatusWorker},
    common::{ChainName, LaneId},
    config::{LaneType, SharedChainConfig, SharedLaneConfig},
    offramp_anomaly_detector::{OffRampAnomaly, OffRampAnomalyDetector},
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CursableAnomaly {
    ManualGlobalCurse(BaseCurseId),
    FinalityViolated {
        finality_violated_chain: ChainName,
        vote_to_curse_on_other_chains_with_legacy_contracts: bool,
    },
    OffRampAnomaly(LaneId, OffRampAnomaly),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CurseType {
    V1_0,
    V1_5(HashSet<CurseSubjectV1_5>),
}

#[derive(Debug, Clone)]
pub struct Curse {
    base_curse_id: BaseCurseId,
    curse_type_by_receiver_chain: HashMap<ChainName, CurseType>,
}

impl Curse {
    fn lane_is_cursed(&self, lane_id: &LaneId, lane_type: &LaneType) -> bool {
        let subject_chain_is_cursed = |receiver_chain, subject_chain| {
            self.curse_type_by_receiver_chain
                .get(&receiver_chain)
                .map(|curse_type| match curse_type {
                    CurseType::V1_0 => true,
                    CurseType::V1_5(curse_subjects) => {
                        curse_subjects.contains(&CurseSubjectV1_5::Chain(subject_chain))
                            || curse_subjects.contains(&CurseSubjectV1_5::GlobalCurseSubject)
                            || (lane_type.is_legacy()
                                && curse_subjects.contains(&CurseSubjectV1_5::LegacyCurseSubject))
                    }
                })
                .unwrap_or(false)
        };
        subject_chain_is_cursed(lane_id.source_chain_name, lane_id.dest_chain_name)
            || subject_chain_is_cursed(lane_id.dest_chain_name, lane_id.source_chain_name)
    }
}

pub type CurseId = Bytes16;

mod base_curse_id {
    use super::CurseId;
    use anyhow::{bail, Result};
    use minieth::bytes::Bytes16;
    use std::{fmt::Display, str::FromStr};

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct BaseCurseId(Bytes16);

    impl BaseCurseId {
        pub fn new(value: Bytes16) -> Result<Self> {
            let first_4_bytes_are_all_zeros = value.as_slice()[..4].iter().all(|byte| *byte == 0u8);
            if !first_4_bytes_are_all_zeros {
                bail!("non-zero values are found in the first 4 bytes of BaseCurseId")
            } else {
                Ok(BaseCurseId(value))
            }
        }
        pub fn random_base_curse_id() -> Self {
            let mut base_curse_id = [0u8; 16];
            openssl::rand::rand_bytes(&mut base_curse_id[4..]).unwrap();
            BaseCurseId::new(base_curse_id.into()).unwrap()
        }
        pub fn to_versioned_curse_id(self, version_number: u32) -> CurseId {
            let mut curse_id: [u8; 16] = self.0.into();
            curse_id[..4].copy_from_slice(&version_number.to_be_bytes());
            curse_id.into()
        }
    }
    impl FromStr for BaseCurseId {
        type Err = anyhow::Error;
        fn from_str(s: &str) -> Result<Self> {
            let bytes16_value = Bytes16::from_str(s)?;
            Self::new(bytes16_value)
        }
    }
    impl Display for BaseCurseId {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "BaseCurseId({})", self.0)
        }
    }
}

pub type BaseCurseId = base_curse_id::BaseCurseId;

#[derive(Debug, Clone)]
pub struct CursableAnomaliesWithVotableCurses {
    afn_type_by_enabled_chain: HashMap<ChainName, AFNType>,
    enabled_lanes_by_source_and_dest_chain:
        HashMap<(ChainName, ChainName), HashSet<(LaneId, LaneType)>>,
    votable_curse_by_cursable_anomaly: HashMap<CursableAnomaly, Curse>,
}

impl CursableAnomaliesWithVotableCurses {
    pub fn new(
        shared_config_by_enabled_chain: HashMap<ChainName, SharedChainConfig>,
        shared_config_by_enabled_lane: HashMap<LaneId, SharedLaneConfig>,
    ) -> Self {
        let mut enabled_lanes_by_source_and_dest_chain: HashMap<_, HashSet<(LaneId, LaneType)>> =
            HashMap::new();
        for (lane_id, shared_lane_config) in shared_config_by_enabled_lane.clone() {
            let source_and_dest_chain = (lane_id.source_chain_name, lane_id.dest_chain_name);
            let lanes = enabled_lanes_by_source_and_dest_chain
                .entry(source_and_dest_chain)
                .or_default();
            lanes.insert((shared_lane_config.lane_id, shared_lane_config.lane_type));
        }
        let afn_type_by_enabled_chain = shared_config_by_enabled_chain
            .into_iter()
            .map(|(chain_name, config)| (chain_name, config.afn_type))
            .collect();
        Self {
            afn_type_by_enabled_chain,
            enabled_lanes_by_source_and_dest_chain,
            votable_curse_by_cursable_anomaly: HashMap::default(),
        }
    }

    pub fn note(&mut self, anomaly: CursableAnomaly) {
        let curse_type_for_global_curse = |afn_type: AFNType| match afn_type {
            AFNType::V1_0 => CurseType::V1_0,
            AFNType::V1_5 => CurseType::V1_5(HashSet::from([CurseSubjectV1_5::GlobalCurseSubject])),
        };
        let lanes_between_chain_pair = |chain_a: ChainName, chain_b: ChainName| {
            let mut lanes_from_a_to_b = self
                .enabled_lanes_by_source_and_dest_chain
                .get(&(chain_a, chain_b))
                .map_or(HashSet::new(), |x| x.clone());
            let lanes_from_b_to_a = self
                .enabled_lanes_by_source_and_dest_chain
                .get(&(chain_b, chain_a))
                .map_or(HashSet::new(), |x| x.clone());
            lanes_from_a_to_b.extend(lanes_from_b_to_a);
            lanes_from_a_to_b
        };
        self.votable_curse_by_cursable_anomaly
            .entry(anomaly.clone())
            .or_insert_with(|| match anomaly {
                CursableAnomaly::ManualGlobalCurse(base_curse_id) => Curse {
                    base_curse_id,
                    curse_type_by_receiver_chain: self
                        .afn_type_by_enabled_chain
                        .iter()
                        .map(|(&chain, &afn_type)| (chain, curse_type_for_global_curse(afn_type)))
                        .collect(),
                },
                CursableAnomaly::OffRampAnomaly(_, _) => Curse {
                    base_curse_id: BaseCurseId::random_base_curse_id(),
                    curse_type_by_receiver_chain: self
                        .afn_type_by_enabled_chain
                        .iter()
                        .map(|(&chain, &afn_type)| (chain, curse_type_for_global_curse(afn_type)))
                        .collect(),
                },
                CursableAnomaly::FinalityViolated {
                    finality_violated_chain : c_bad ,
                    vote_to_curse_on_other_chains_with_legacy_contracts,
                } => Curse {
                    base_curse_id: BaseCurseId::random_base_curse_id(),
                    curse_type_by_receiver_chain: self
                        .afn_type_by_enabled_chain
                        .iter()
                        .filter_map(|(&chain, &afn_type)| {
                            if chain == c_bad {
                                Some((chain, curse_type_for_global_curse(afn_type)))
                            } else {
                                let lanes = lanes_between_chain_pair(chain, c_bad);
                                if !lanes.is_empty() {
                                    match afn_type {
                                        AFNType::V1_0 => {
                                            if vote_to_curse_on_other_chains_with_legacy_contracts {
                                                Some((chain, CurseType::V1_0))
                                            } else {
                                                None
                                            }
                                        }
                                        AFNType::V1_5 => {
                                            let has_legacy_lane = lanes
                                                .iter()
                                                .any(|(_lane_id, lane_type)| lane_type.is_legacy());
                                            if !has_legacy_lane {
                                                Some((chain,  CurseType::V1_5(HashSet::from([CurseSubjectV1_5::Chain(c_bad)]))))
                                            } else if vote_to_curse_on_other_chains_with_legacy_contracts {
                                                Some((chain,
                                                    CurseType::V1_5(HashSet::from([
                                                        CurseSubjectV1_5::Chain(c_bad),
                                                        CurseSubjectV1_5::LegacyCurseSubject,
                                                    ]))
                                                ))
                                            } else {
                                                None
                                            }
                                        }
                                    }
                                } else {
                                    None
                                }
                            }
                        })
                        .collect(),
                },
            });
    }

    pub fn is_cursed(&self) -> bool {
        !self.votable_curse_by_cursable_anomaly.is_empty()
    }

    pub fn lane_is_cursed(&self, lane_id: &LaneId, lane_type: &LaneType) -> bool {
        self.votable_curse_by_cursable_anomaly
            .iter()
            .any(|(_anomaly, curse)| curse.lane_is_cursed(lane_id, lane_type))
    }

    pub fn get_anomalies_and_curse_parameters_for_chain(
        &self,
        chain: ChainName,
    ) -> Vec<(CursableAnomaly, BaseCurseId, CurseType)> {
        self.votable_curse_by_cursable_anomaly
            .iter()
            .filter_map(|(anomaly, curse)| {
                curse
                    .curse_type_by_receiver_chain
                    .get(&chain)
                    .map(|curse_type| (anomaly.clone(), curse.base_curse_id, curse_type.clone()))
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct CurseBeacon {
    pub chain_status_workers: HashMap<ChainName, Arc<ChainStatusWorker>>,
    pub offramp_anomaly_detectors: HashMap<LaneId, Arc<OffRampAnomalyDetector>>,

    pub upon_finality_violation_vote_to_curse_on_other_chains_with_legacy_contracts:
        HashSet<ChainName>,

    pub cursable_anomalies_with_votable_curses: Arc<RwLock<CursableAnomaliesWithVotableCurses>>,
}

impl CurseBeacon {
    pub fn update(&self) {
        let mut local_cursable_anomalies_with_votable_curses = self
            .cursable_anomalies_with_votable_curses
            .read()
            .unwrap()
            .clone();

        for (chain_name, chain_status_worker) in &self.chain_status_workers {
            if let Some(ChainStatus::FinalityViolated) = chain_status_worker.latest_chain_status() {
                local_cursable_anomalies_with_votable_curses
                    .note(CursableAnomaly::FinalityViolated {
                    finality_violated_chain: *chain_name,
                    vote_to_curse_on_other_chains_with_legacy_contracts: self
                        .upon_finality_violation_vote_to_curse_on_other_chains_with_legacy_contracts
                        .contains(chain_name),
                });
            }
        }

        for (lane_id, offramp_anomaly_detector) in &self.offramp_anomaly_detectors {
            let state = offramp_anomaly_detector.state.read().unwrap();
            state
                .anomalies
                .iter()
                .map(|offramp_anomaly| {
                    CursableAnomaly::OffRampAnomaly(lane_id.clone(), *offramp_anomaly)
                })
                .for_each(|anomaly| local_cursable_anomalies_with_votable_curses.note(anomaly));
        }

        *self.cursable_anomalies_with_votable_curses.write().unwrap() =
            local_cursable_anomalies_with_votable_curses.clone();
    }

    pub fn is_cursed(&self) -> bool {
        self.cursable_anomalies_with_votable_curses
            .read()
            .unwrap()
            .is_cursed()
    }

    pub fn lane_is_cursed(&self, lane_id: &LaneId, lane_type: &LaneType) -> bool {
        self.cursable_anomalies_with_votable_curses
            .read()
            .unwrap()
            .lane_is_cursed(lane_id, lane_type)
    }

    pub fn get_anomalies_and_curse_parameters_for_chain(
        &self,
        chain: ChainName,
    ) -> Vec<(CursableAnomaly, BaseCurseId, CurseType)> {
        self.cursable_anomalies_with_votable_curses
            .read()
            .unwrap()
            .get_anomalies_and_curse_parameters_for_chain(chain)
    }
}
