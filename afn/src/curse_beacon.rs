use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use minieth::bytes::Bytes32;

use crate::{
    chain_status::ChainStatus,
    chain_status_worker::ChainStatusWorker,
    common::{ChainName, LaneId},
    offramp_anomaly_detector::{OffRampAnomaly, OffRampAnomalyDetector},
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CursableAnomaly {
    ManualCurse(CurseId),
    FinalityViolated(ChainName),
    OffRampAnomaly(LaneId, OffRampAnomaly),
}

pub type CurseId = Bytes32;

#[derive(Default, Debug, Clone)]
pub struct CursableAnomaliesWithCurseIds(HashMap<CursableAnomaly, CurseId>);

impl CursableAnomaliesWithCurseIds {
    pub fn note(&mut self, anomaly: CursableAnomaly) {
        self.0
            .entry(anomaly.clone())
            .or_insert_with(|| match anomaly {
                CursableAnomaly::ManualCurse(curse_id) => curse_id,
                _ => {
                    let mut curse_id = [0u8; 32];
                    openssl::rand::rand_bytes(&mut curse_id).unwrap();
                    curse_id.into()
                }
            });
    }
}

#[derive(Default, Debug, Clone)]
pub struct CurseBeacon {
    pub chain_status_workers: HashMap<ChainName, Arc<ChainStatusWorker>>,
    pub offramp_anomaly_detectors: HashMap<LaneId, Arc<OffRampAnomalyDetector>>,

    pub cursable_anomalies: Arc<RwLock<CursableAnomaliesWithCurseIds>>,
}

impl CurseBeacon {
    pub fn update(&self) {
        let mut local_cursable_anomalies = self.cursable_anomalies.read().unwrap().clone();

        for (chain_name, chain_status_worker) in &self.chain_status_workers {
            if let Some(ChainStatus::FinalityViolated) = chain_status_worker.latest_chain_status() {
                local_cursable_anomalies.note(CursableAnomaly::FinalityViolated(*chain_name));
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
                .for_each(|anomaly| local_cursable_anomalies.note(anomaly));
        }

        *self.cursable_anomalies.write().unwrap() = local_cursable_anomalies.clone();
    }

    pub fn is_cursed(&self) -> bool {
        !self.cursable_anomalies.read().unwrap().0.is_empty()
    }

    pub fn get_anomalies_and_curse_ids(&self) -> Vec<(CursableAnomaly, CurseId)> {
        self.cursable_anomalies
            .read()
            .unwrap()
            .0
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect()
    }
}
