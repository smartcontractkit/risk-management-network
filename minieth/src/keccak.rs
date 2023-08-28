pub use tiny_keccak::{Hasher, Keccak};

pub fn keccak256(input: &[u8]) -> [u8; 32] {
    let mut k = Keccak::v256();
    let mut output = [0u8; 32];
    k.update(input);
    k.finalize(&mut output);
    output
}
