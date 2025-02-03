use anyhow::{anyhow, Result};
use minieth::bytes::{Address, Bytes32};

pub type Ed25519PublicKey = Bytes32;
pub type Ed25519SecretKey = [u8; 32];
pub type Ed25519Signature = [u8; 64];

pub fn sign_ed25519(secret_key: &Ed25519SecretKey, msg: &[u8]) -> Result<Ed25519Signature> {
    let pkey = openssl::pkey::PKey::private_key_from_raw_bytes(
        secret_key.as_slice(),
        openssl::pkey::Id::ED25519,
    )?;
    let mut signer = openssl::sign::Signer::new_without_digest(&pkey)?;
    let v = signer.sign_oneshot_to_vec(msg)?;
    v.try_into()
        .map_err(|_| anyhow!("unexpected signature size"))
}

pub fn verify_ed25519(
    public_key: &Ed25519PublicKey,
    msg: &[u8],
    sig: &Ed25519Signature,
) -> Result<()> {
    let pkey = openssl::pkey::PKey::public_key_from_raw_bytes(
        public_key.as_slice(),
        openssl::pkey::Id::ED25519,
    )?;
    let mut verifier = openssl::sign::Verifier::new_without_digest(&pkey)?;
    if verifier.verify_oneshot(sig.as_slice(), msg)? {
        Ok(())
    } else {
        Err(anyhow!("does not verify"))
    }
}

pub type EcdsaSecretKey = minieth::ecdsa::SecretKey;
pub type EcdsaPublicKey = Address;

#[derive(Debug, Clone, Copy)]
pub struct EcdsaSignature {
    pub r: Bytes32,
    pub s: Bytes32,
}
