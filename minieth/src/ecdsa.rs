use std::str::FromStr;

use secp256k1::ecdsa::RecoverableSignature;
use secp256k1::rand::rngs::OsRng;
use secp256k1::{generate_keypair, Message, SECP256K1};
pub use secp256k1::{PublicKey, SecretKey};

use crate::bytes::{Address, Bytes, Bytes32};
use crate::keccak::keccak256;
use crate::rlp::{RlpEncodable, RlpStream};
use crate::u256::U256;

impl From<PublicKey> for Address {
    fn from(value: PublicKey) -> Self {
        let array: [u8; 20] = keccak256(&value.serialize_uncompressed()[1..])[32 - 20..]
            .try_into()
            .unwrap();
        array.into()
    }
}

impl TryFrom<Bytes32> for SecretKey {
    type Error = secp256k1::Error;
    fn try_from(value: Bytes32) -> Result<Self, Self::Error> {
        SecretKey::from_slice(value.as_slice())
    }
}

impl TryFrom<Bytes> for PublicKey {
    type Error = secp256k1::Error;
    fn try_from(value: Bytes) -> Result<Self, Self::Error> {
        PublicKey::from_slice(value.as_ref())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct EthereumSignature {
    v: U256,
    r: U256,
    s: U256,
}

impl RlpEncodable for EthereumSignature {
    fn to_rlp_bytes(&self) -> Vec<u8> {
        let mut rlp = RlpStream::new();
        rlp.push(&self.v);
        rlp.push(&self.r);
        rlp.push(&self.s);
        rlp.finalize_without_length_prefix()
    }
}

impl EthereumSignature {
    pub(crate) fn to_legacy(self, chain_id: u64) -> EthereumSignature {
        match TryInto::<u64>::try_into(self.v) {
            Ok(v) => match v {
                0 | 1 => EthereumSignature {
                    v: (chain_id * 2 + 35 + v).into(),
                    r: self.r,
                    s: self.s,
                },
                _ => self,
            },
            Err(_) => self,
        }
    }
}

pub trait EthereumSigner: std::fmt::Debug + Sync + Send {
    fn sign_hashed(&self, hashed: [u8; 32]) -> EthereumSignature;
    fn sign(&self, data: &[u8]) -> EthereumSignature;
    fn address(&self) -> Address;
}

#[derive(Debug)]
pub struct LocalSigner {
    secret_key: SecretKey,
}

pub fn generate_secret_key() -> SecretKey {
    let (secret_key, _) = generate_keypair(&mut OsRng);
    secret_key
}

pub fn secret_key_to_public_key(sk: &SecretKey) -> PublicKey {
    sk.public_key(SECP256K1)
}

impl LocalSigner {
    pub fn generate() -> Self {
        let secret_key = generate_secret_key();
        Self { secret_key }
    }

    pub fn from_secret_key(secret_key: SecretKey) -> Self {
        Self { secret_key }
    }
}

impl FromStr for LocalSigner {
    type Err = <SecretKey as FromStr>::Err;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let secret_key = SecretKey::from_str(s)?;
        Ok(Self::from_secret_key(secret_key))
    }
}

impl EthereumSigner for LocalSigner {
    fn sign_hashed(&self, hashed: [u8; 32]) -> EthereumSignature {
        let message = Message::from_digest(hashed);
        let sig = SECP256K1.sign_ecdsa_recoverable(&message, &self.secret_key);
        sig.into()
    }

    fn sign(&self, data: &[u8]) -> EthereumSignature {
        self.sign_hashed(keccak256(data))
    }

    fn address(&self) -> Address {
        secret_key_to_public_key(&self.secret_key).into()
    }
}

impl From<RecoverableSignature> for EthereumSignature {
    fn from(sig: RecoverableSignature) -> Self {
        let (recovery_id, serialized_sig) = sig.serialize_compact();
        let v = U256::new(recovery_id.to_i32() as u128);
        let (r_bytes, s_bytes) = (&serialized_sig[..32], &serialized_sig[32..]);
        let r = U256::from_be_bytes(r_bytes.try_into().unwrap());
        let s = U256::from_be_bytes(s_bytes.try_into().unwrap());
        Self { v, r, s }
    }
}
