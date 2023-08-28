use crate::commit_store::Interval;
use minieth::bytes::Bytes32;

pub trait OnRampReader {
    fn get_message_hashes(&self, interval: &Interval) -> Option<Vec<Bytes32>>;
}
