use minieth::{
    bytes::Bytes32,
    keccak::{Hasher, Keccak},
};
use std::collections::VecDeque;

pub const LEAF_DOMAIN_SEPARATOR: &[u8; 32] = b"\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00";
pub const INTERNAL_NODE_DOMAIN_SEPARATOR: &[u8; 32] = b"\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x01";

const ZERO_HASH: &[u8; 32] = b"\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff";

fn hash_node<'a>(left: &'a [u8; 32], right: &'a [u8; 32]) -> [u8; 32] {
    let mut output = [0u8; 32];
    let mut hasher = Keccak::v256();
    hasher.update(INTERNAL_NODE_DOMAIN_SEPARATOR);
    hasher.update(left);
    hasher.update(right);
    hasher.finalize(&mut output);
    output
}

pub fn root_from_hashed_leafs(hashed_leafs: &[Bytes32]) -> Option<Bytes32> {
    let mut level: VecDeque<[u8; 32]> = hashed_leafs
        .iter()
        .map(|hl| <[u8; 32]>::from(*hl))
        .collect();
    while level.len() > 1 {
        let len = level.len();
        let mut pops = 0;
        while pops < len {
            let a = level.pop_front().unwrap();
            pops += 1;
            let b = if pops != len {
                pops += 1;
                level.pop_front().unwrap()
            } else {
                *ZERO_HASH
            };

            let (left, right) = if a < b { (a, b) } else { (b, a) };
            level.push_back(hash_node(&left, &right))
        }
    }
    level.pop_front().map(|root| root.into())
}
