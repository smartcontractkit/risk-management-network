use crate::{
    chain_status_worker::{
        ChainStatusUpdater, ChainStatusWorker, ConfirmationDepthChainStatusUpdater,
        FinalityTagChainStatusUpdater,
    },
    common::ChainName,
    config::{ChainStability, SharedChainConfig},
    metrics::{ChainStatusMetricsHandle, GasFeeMetricsHandle, GasFeeMetricsUpdateWorker},
    worker::{self, ShutdownHandleGroup},
};
use anyhow::Result;
use minieth::rpc::Rpc;
use std::{boxed::Box, sync::Arc};

use super::onchain_config_discovery_worker::OnchainConfigDiscoveryWorker;

pub struct ChainState {
    pub name: ChainName,
    pub chain_status_worker: Arc<ChainStatusWorker>,
    pub config_discovery_worker: Arc<OnchainConfigDiscoveryWorker>,
    pub gas_fee_metrics_update_worker: Arc<GasFeeMetricsUpdateWorker>,
}

impl ChainState {
    pub fn new_and_spawn_workers(
        ctx: &Arc<worker::Context>,
        rpc: Arc<Rpc>,
        config: &SharedChainConfig,
        chain_status_metrics_handle: ChainStatusMetricsHandle,
        gas_fee_metrics_handle: GasFeeMetricsHandle,
    ) -> Result<(Self, ShutdownHandleGroup)> {
        let mut shutdown_handles = ShutdownHandleGroup::default();
        let chain_status_worker = {
            let chain_status_updater: Box<dyn ChainStatusUpdater + Send> = {
                let rpc = Arc::clone(&rpc);
                match config.stability {
                    ChainStability::ConfirmationDepth {
                        soft_confirmations,
                        hard_confirmations,
                    } => Box::new(ConfirmationDepthChainStatusUpdater {
                        soft_confirmations: soft_confirmations as usize,
                        hard_confirmations: hard_confirmations as usize,
                        rpc,
                        name: config.name,
                    }),
                    ChainStability::FinalityTag {
                        soft_confirmations: _,
                    } => Box::new(FinalityTagChainStatusUpdater {
                        name: config.name,
                        rpc,
                    }),
                }
            };
            shutdown_handles.add(ChainStatusWorker::spawn(
                ctx,
                config.name,
                crate::config::CHAIN_STATUS_WORKER_POLL_INTERVAL,
                chain_status_updater,
                chain_status_metrics_handle,
            ))
        };
        let config_discovery_worker = shutdown_handles.add(OnchainConfigDiscoveryWorker::spawn(
            ctx,
            Arc::clone(&rpc),
            config,
            crate::config::ONCHAIN_CONFIG_DISCOVERY_WORKER_POLL_INTERVAL,
        )?);
        let gas_fee_metrics_update_worker = shutdown_handles.add(GasFeeMetricsUpdateWorker::spawn(
            Arc::clone(ctx),
            config.to_owned(),
            rpc,
            gas_fee_metrics_handle,
        )?);
        Ok((
            Self {
                name: config.name,
                chain_status_worker: Arc::new(chain_status_worker),
                config_discovery_worker: Arc::new(config_discovery_worker),
                gas_fee_metrics_update_worker: Arc::new(gas_fee_metrics_update_worker),
            },
            shutdown_handles,
        ))
    }
}
