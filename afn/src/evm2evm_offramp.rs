use anyhow::anyhow;
use miniabi::abi_decode::AbiDecode;
use minieth::{
    bytes::{Bytes, Bytes32},
    rpc::EVMLog,
};

use crate::common::{LogSignature, UncheckedDecodeLog};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MessageExecutionState {
    Untouched,
    InProgress,
    Success,
    Failure,
}

pub type MessageId = Bytes32;

#[derive(Debug, Clone, Copy)]
pub struct ExecutionStateChanged {
    pub sequence_number: u64,
    pub message_id: MessageId,
    pub state: MessageExecutionState,
}

impl LogSignature for ExecutionStateChanged {
    fn log_signature() -> &'static str {
        "ExecutionStateChanged(uint64,bytes32,uint8,bytes)"
    }
}

impl UncheckedDecodeLog for ExecutionStateChanged {
    fn unchecked_decode_log(log: EVMLog) -> anyhow::Result<Self> {
        let [sequence_number, message_id]: [Bytes32; 2] = log.topics[1..]
            .try_into()
            .map_err(|topics| anyhow!("unexpected topics {:?}", topics))?;
        let (state, _return_data) = <(u8, Bytes)>::abi_decode(log.data)?;
        Ok(Self {
            sequence_number: u64::abi_decode(sequence_number)?,
            message_id,
            state: match state {
                0 => Ok(MessageExecutionState::Untouched),
                1 => Ok(MessageExecutionState::InProgress),
                2 => Ok(MessageExecutionState::Success),
                3 => Ok(MessageExecutionState::Failure),
                _ => Err(anyhow!("unknown state {state}")),
            }?,
        })
    }
}
