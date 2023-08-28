use crate::bytes::{Address, Bytes, Bytes32};
use crate::rpc::{BlockIdentifier, RpcError};
use crate::tx::LegacyTransactionRequest;
use crate::u256::U256;
use crate::{ecdsa::EthereumSigner, rpc::Rpc, tx::Eip1559TransactionRequest};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone, Copy)]
pub enum FeeUnit {
    Wei(u64),
    Kwei(u64),
    Mwei(u64),
    Gwei(u64),
}

impl From<FeeUnit> for U256 {
    fn from(value: FeeUnit) -> Self {
        match value {
            FeeUnit::Wei(wei) => wei.into(),
            FeeUnit::Kwei(kwei) => (kwei as u128 * 1_000).into(),
            FeeUnit::Mwei(mwei) => (mwei as u128 * 1_000_000).into(),
            FeeUnit::Gwei(gwei) => (gwei as u128 * 1_000_000_000).into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum TransactionSenderFeeConfig {
    Eip1559 {
        max_fee_per_gas: FeeUnit,
        max_priority_fee_per_gas: FeeUnit,
    },
    Legacy {
        gas_price: FeeUnit,
    },
}

#[derive(Debug)]
pub struct TransactionSender {
    chain_id: u64,
    fee_config: TransactionSenderFeeConfig,

    rpc: Arc<Rpc>,
    signer: Arc<dyn EthereumSigner>,

    previous_transaction_count_with_timestamp: Option<(u64, Instant)>,
    nonce: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct TransactionRequest {
    pub gas: u64,
    pub to: Address,
    pub value: U256,
    pub data: Bytes,
}

impl TransactionSender {
    pub fn new(
        chain_id: u64,
        fee_config: TransactionSenderFeeConfig,
        rpc: Arc<Rpc>,
        signer: Arc<dyn EthereumSigner>,
    ) -> Self {
        Self {
            chain_id,
            fee_config,
            rpc,
            signer,
            previous_transaction_count_with_timestamp: None,
            nonce: None,
        }
    }
    pub fn send(&mut self, tx_req: TransactionRequest) -> Result<Bytes32, RpcError> {
        let start = Instant::now();
        const MAX_BLOCKING_TIME: Duration = Duration::from_secs(120);
        const LATEST_TRANSACTION_COUNT_STALL_THRESHOLD: Duration = Duration::from_secs(20);
        const MAX_TXS_INFLIGHT: u64 = 16;
        const ETH_ESTIMATEGAS_OVERESTIMATE_PERCENT: u64 = 10;

        let can_retry = || start.elapsed() < MAX_BLOCKING_TIME;
        let remaining_time = || MAX_BLOCKING_TIME.saturating_sub(start.elapsed());

        loop {
            let tx_req = tx_req.clone();

            let latest_transaction_count = self
                .rpc
                .get_transaction_count(self.signer.address(), BlockIdentifier::Latest)?;
            let pending_transaction_count = self
                .rpc
                .get_transaction_count(self.signer.address(), BlockIdentifier::Pending)?;
            let txs_inflight = pending_transaction_count.saturating_sub(latest_transaction_count);
            log::trace!(
                "txs inflight: {}, chain id: {}, from: {}",
                txs_inflight,
                self.chain_id,
                self.signer.address()
            );

            if let Some((previous_transaction_count, previous_timestamp)) =
                self.previous_transaction_count_with_timestamp
            {
                if previous_transaction_count > latest_transaction_count
                    || (previous_transaction_count == latest_transaction_count
                        && previous_timestamp.elapsed() > LATEST_TRANSACTION_COUNT_STALL_THRESHOLD)
                {
                    self.nonce = None;
                    self.previous_transaction_count_with_timestamp = None;
                } else if previous_transaction_count < latest_transaction_count {
                    self.previous_transaction_count_with_timestamp = None;
                }
            }

            if self.previous_transaction_count_with_timestamp.is_none() {
                self.previous_transaction_count_with_timestamp =
                    Some((latest_transaction_count, Instant::now()));
            }

            let nonce = match self.nonce {
                Some(nonce) => {
                    if txs_inflight > MAX_TXS_INFLIGHT {
                        log::warn!(
                            "skipping sending tx and retrying, too many txs inflight (nonces: {}..={}, total: {}, max: {}) chain id: {}, from: {}, remaining time: {:?}",
                            latest_transaction_count,
                            pending_transaction_count,
                            txs_inflight,
                            MAX_TXS_INFLIGHT,
                            self.chain_id,
                            self.signer.address(),
                            remaining_time(),
                        );
                        continue;
                    } else {
                        nonce
                    }
                }
                None => latest_transaction_count,
            };

            let gas = match self.rpc.estimate_gas(
                self.signer.address(),
                tx_req.to,
                tx_req.data.clone(),
                BlockIdentifier::Pending,
            ) {
                Ok(gas_estimate) => std::cmp::max(
                    gas_estimate.saturating_mul(100 + ETH_ESTIMATEGAS_OVERESTIMATE_PERCENT) / 100,
                    tx_req.gas,
                ),
                Err(err) => {
                    log::error!("eth_estimateGas for tx request: {tx_req:?}, chain id: {}, from: {}, failed with error: {err:?}", self.chain_id, self.signer.address());
                    if can_retry() {
                        continue;
                    } else {
                        return Err(err);
                    }
                }
            };

            log::debug!(
                "using gas {gas} for tx request: {tx_req:?}, chain id: {}, from: {}",
                self.chain_id,
                self.signer.address(),
            );
            let gas = gas.into();
            let tx_bytes = match self.fee_config {
                TransactionSenderFeeConfig::Eip1559 {
                    max_fee_per_gas,
                    max_priority_fee_per_gas,
                } => Eip1559TransactionRequest {
                    chain_id: self.chain_id,
                    access_list: vec![],
                    data: tx_req.data,
                    gas,
                    max_fee_per_gas: max_fee_per_gas.into(),
                    max_priority_fee_per_gas: max_priority_fee_per_gas.into(),
                    nonce: nonce.into(),
                    to: tx_req.to,
                    value: tx_req.value,
                }
                .sign(self.signer.as_ref())
                .to_rlp_bytes(),
                TransactionSenderFeeConfig::Legacy { gas_price } => LegacyTransactionRequest {
                    chain_id: self.chain_id,
                    data: tx_req.data,
                    gas,
                    gas_price: gas_price.into(),
                    nonce: nonce.into(),
                    to: tx_req.to,
                    value: tx_req.value,
                }
                .sign(self.signer.as_ref())
                .to_rlp_bytes(),
            };
            match self.rpc.send_raw_transaction(tx_bytes.into()) {
                Ok(txid) => {
                    log::debug!(
                        "used nonce: {}, chain id: {}, from: {}",
                        nonce,
                        self.chain_id,
                        self.signer.address(),
                    );
                    self.nonce.replace(nonce + 1);
                    break Ok(txid);
                }
                Err(RpcError::ErrorResponse(err)) if can_retry() => {
                    let new_nonce = self
                        .rpc
                        .get_transaction_count(self.signer.address(), BlockIdentifier::Pending)?;
                    self.nonce.replace(new_nonce);
                    log::warn!(
                        "attempt to send transaction with chain id: {}, from: {}, nonce: {nonce}, remaining time: {:?} failed with retriable error: {err:?}, new nonce: {new_nonce}",
                        self.chain_id,
                        self.signer.address(),
                        remaining_time(),
                    );
                }
                Err(err) => {
                    break Err(err);
                }
            }
        }
    }
}
