use crate::{
    afn_contract::{AFNInterface, VersionedOnchainConfig},
    common::ChainName,
    config::SharedChainConfig,
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
        config: &SharedChainConfig,
        poll_interval: std::time::Duration,
    ) -> Result<(Self, ShutdownHandle)> {
        let worker_name = format!(
            "ConfigDiscoveryWorker({},{})",
            config.name, config.afn_contract
        );
        let versioned_onchain_config = Arc::new(RwLock::new(None));

        let handle = ctx.spawn(worker_name, {
            let config = config.clone();
            let versioned_onchain_config = Arc::clone(&versioned_onchain_config);
            move |ctx, worker_name| -> Result<()> {
                let worker_name = worker_name.to_owned();
                let afn_contract = AFNInterface::create_from_chain_config(rpc, &config)?;
                ctx.repeat(worker_name, poll_interval, move |_ctx| {
                    let new_versioned_onchain_config =
                        afn_contract.get_versioned_onchain_config()?;
                    *versioned_onchain_config.write().unwrap() = Some(new_versioned_onchain_config);
                    Ok(())
                })
            }
        });
        Ok((
            Self {
                chain_name: config.name,
                afn_address: config.afn_contract,
                versioned_onchain_config,
            },
            handle,
        ))
    }
    pub fn latest(&self) -> Option<VersionedOnchainConfig> {
        self.versioned_onchain_config.read().unwrap().to_owned()
    }
}
