use crate::contracts::onramp::CcipMessageSent;
use crate::contracts::prelude::*;
use crate::offchain::ClosedInterval;
use afn::common::DecodeLog;
use afn::common::LogTopic;
use afn::contract_event_state_machine::ContractEventFilter;
use afn::contract_event_state_machine::State;
use afn::contract_event_state_machine::UnstableContractEventStateMachineWorker;
use anyhow::bail;
use anyhow::Context as _;
use minieth::bytes::Bytes32;

#[derive(Clone, Copy, Debug)]
pub struct SequencedMessageHash {
    pub number: u64,
    pub hash: Bytes32,
}

#[derive(Clone, Debug, Default)]
pub struct SourceBlessState {
    pub sequenced_message_hashes_by_dest:
        std::collections::HashMap<ChainSelector, std::collections::VecDeque<SequencedMessageHash>>,
}

pub struct SingleEventFilter<T: DecodeLog + Send + Sync> {
    _phantom: std::marker::PhantomData<T>,
}

impl<T: DecodeLog + Send + Sync> ContractEventFilter for SingleEventFilter<T> {
    type ContractEventUnion = T;

    fn log_topics(&self) -> Vec<Bytes32> {
        vec![T::log_topic()]
    }

    fn decode_log(&self, log: minieth::rpc::EVMLog) -> anyhow::Result<Self::ContractEventUnion> {
        T::decode_log(log)
    }
}

impl<T: DecodeLog + Send + Sync> Default for SingleEventFilter<T> {
    fn default() -> Self {
        Self {
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<T: DecodeLog + Send + Sync> Clone for SingleEventFilter<T> {
    fn clone(&self) -> Self {
        Self {
            _phantom: std::marker::PhantomData,
        }
    }
}

impl State for SourceBlessState {
    type ContractEventUnion = CcipMessageSent;
    type ContractEventFilter = SourceBlessEventFilter;
    fn update_with_event(
        &mut self,
        _block_number: u64,
        _block_hash: Bytes32,
        onramp_address: Address,
        _filter: &Self::ContractEventFilter,
        ev: CcipMessageSent,
    ) -> Result<()> {
        let CcipMessageSent {
            dest_chain_selector,
            evm2any_ramp_message,
            sequence_number: ev_sequence_number,
        } = ev;

        let sequenced_message_hashes = self
            .sequenced_message_hashes_by_dest
            .entry(dest_chain_selector)
            .or_default();

        let message_sequence_number = evm2any_ramp_message.number();

        if ev_sequence_number != message_sequence_number {
            bail!(
                "sequence number mismatch: Evm2AnyRamp {} vs CcipMessageSent {}",
                message_sequence_number,
                ev_sequence_number
            );
        }

        let new_seq_msg_hash = SequencedMessageHash {
            number: ev_sequence_number,
            hash: crate::contracts::onramp::Any2EvmRampMessageToHash {
                any2evm_ramp_message: evm2any_ramp_message
                    .try_into()
                    .context("evm2any -> any2evm conversion failed")?,
                onramp_address,
            }
            .hash(),
        };

        if let Some(old_seq_msg_hash) = sequenced_message_hashes.back() {
            if old_seq_msg_hash.number + 1 != new_seq_msg_hash.number {
                bail!("last sequenced message hash is {:?} and we're trying to push {:?} which is not contiguous",
                        old_seq_msg_hash, new_seq_msg_hash);
            }
        }

        tracing::trace!(
            "pushing new onchain sequenced message hash {:?}",
            new_seq_msg_hash
        );

        sequenced_message_hashes.push_back(new_seq_msg_hash);
        Ok(())
    }
}

impl SourceBlessState {
    pub fn merkle_root(
        &self,
        dest_chain_selector: ChainSelector,
        closed_interval: ClosedInterval,
    ) -> Option<Bytes32> {
        let sequenced_message_hashes = self
            .sequenced_message_hashes_by_dest
            .get(&dest_chain_selector)?;

        let initial_number = {
            let SequencedMessageHash { number, .. } = sequenced_message_hashes.front()?;
            *number
        };

        let start_index =
            (closed_interval.min_msg_nr as usize).checked_sub(initial_number as usize)?;
        let end_index = start_index.checked_add(
            closed_interval
                .max_msg_nr
                .checked_sub(closed_interval.min_msg_nr)? as usize,
        )?;

        if end_index >= sequenced_message_hashes.len() {
            None
        } else {
            afn::merkle::root_from_hashed_leafs(
                &sequenced_message_hashes
                    .range(start_index..=end_index)
                    .map(|SequencedMessageHash { hash, .. }| *hash)
                    .collect::<Vec<_>>(),
            )
        }
    }
}

pub type SourceBlessWorker = UnstableContractEventStateMachineWorker<SourceBlessState>;
pub type SourceBlessEventFilter = SingleEventFilter<CcipMessageSent>;
