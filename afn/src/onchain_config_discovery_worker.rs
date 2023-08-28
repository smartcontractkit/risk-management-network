use crate::{
    afn_contract::{AFNInterface, VersionedOnchainConfig},
    common::ChainName,
    config::ChainConfig,
    worker::{Context, ShutdownHandle},
};
use anyhow::Result;
use minieth::{bytes::Address, rpc::Rpc};
use std::sync::{Arc, RwLock};

pub struct OnchainConfigDiscoveryWorker {
    pub chain_name: ChainName,
    pub afn_address: Address,
    versioned_onchain_config: Arc<RwLock<Option<VersionedOnchainConfig>>>,
}

impl OnchainConfigDiscoveryWorker {
    pub fn spawn(
        ctx: &Arc<Context>,
        rpc: Arc<Rpc>,
        config: &ChainConfig,
        poll_interval: std::time::Duration,
    ) -> Result<(Self, ShutdownHandle)> {
        let worker_name = format!(
            "ConfigDiscoveryWorker({},{})",
            config.name, config.afn_contract
        );
        let config_details = Arc::new(RwLock::new(None));

        let handle = ctx.spawn(worker_name, {
            let config = config.clone();
            let config_details = Arc::clone(&config_details);
            move |ctx, worker_name| -> Result<()> {
                let afn_contract = AFNInterface::create_from_chain_config(rpc, &config)?;
                let worker_name = worker_name.to_owned();
                ctx.repeat(poll_interval, move |_ctx| {
                    let new_config_details = {
                        match afn_contract.get_versioned_onchain_config() {
                            Ok(versioned_onchain_config) => Some(versioned_onchain_config),
                            Err(e) => {
                                tracing::error!(
                                    "{worker_name}: failed to get config details (error: {e:?})",
                                );
                                None
                            }
                        }
                    };
                    *config_details.write().unwrap() = new_config_details;
                    Ok(())
                })
            }
        });
        Ok((
            Self {
                chain_name: config.name,
                afn_address: config.afn_contract,
                versioned_onchain_config: config_details,
            },
            handle,
        ))
    }
    pub fn latest(&self) -> Option<VersionedOnchainConfig> {
        self.versioned_onchain_config.read().unwrap().to_owned()
    }
}
