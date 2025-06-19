use crate::keystore_proto;
use crate::signing::{
    sign_ed25519, EcdsaPublicKey, EcdsaSecretKey, EcdsaSignature, Ed25519PublicKey,
    Ed25519SecretKey, Ed25519Signature,
};
use afn::encryption::EncryptedSecrets;
use anyhow::{anyhow, Context as _, Result};
use minieth::ecdsa::EthereumSigner as _;
use prost::Message;
use serde::Serialize;

pub trait Keystore {
    fn sign_offchain(&self, msg: &[u8]) -> Ed25519Signature;
    fn sign_evm_onchain(&self, msg: &[u8]) -> EcdsaSignature;
    fn as_public(&self) -> PublicKeys;
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct PublicKeys {
    pub offchain_public_key: Ed25519PublicKey,
    pub evm_onchain_public_key: EcdsaPublicKey,
}

#[derive(Clone, Copy)]
pub struct LocalKeystore {
    pub offchain_secret_key: Ed25519SecretKey,
    pub evm_onchain_secret_key: EcdsaSecretKey,
}

impl LocalKeystore {
    pub fn generate() -> Self {
        Self {
            offchain_secret_key: openssl::pkey::PKey::generate_ed25519()
                .unwrap()
                .raw_private_key()
                .unwrap()
                .try_into()
                .unwrap(),
            evm_onchain_secret_key: minieth::ecdsa::generate_secret_key(),
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        keystore_proto::Keystore::from(*self).encode_to_vec()
    }

    pub fn decode(encoded: &[u8]) -> Result<Self> {
        keystore_proto::Keystore::decode(encoded)?.try_into()
    }

    pub fn encrypt(&self, passphrase: &str) -> Result<EncryptedSecrets> {
        EncryptedSecrets::encrypt(
            &self.encode(),
            passphrase,
            &serde_json::to_string(&self.as_public())?,
        )
    }

    pub fn decrypt(encrypted: &EncryptedSecrets, passphrase: &str) -> Result<Self> {
        Self::decode(&encrypted.decrypt(passphrase)?)
    }

    pub fn encrypt_raw(&self, passphrase: &str) -> Result<Vec<u8>> {
        let encrypted = self.encrypt(passphrase)?;
        Ok(serde_json::to_vec_pretty(&encrypted)?)
    }

    pub fn decrypt_raw(encrypted_encoded: &[u8], passphrase: &str) -> Result<Self> {
        let encrypted = serde_json::from_slice(encrypted_encoded)?;
        Self::decrypt(&encrypted, passphrase)
    }

    pub fn decrypt_file(path: impl AsRef<std::path::Path>, passphrase: &str) -> Result<Self> {
        Self::decrypt_raw(
            &std::fs::read(path).context("while decrypting keystore file")?,
            passphrase,
        )
    }

    pub fn encrypt_file(&self, path: impl AsRef<std::path::Path>, passphrase: &str) -> Result<()> {
        if path.as_ref().exists() {
            return Err(anyhow!("file already exists"));
        }
        std::fs::write(path, self.encrypt_raw(passphrase)?).context("while writing keystore file")
    }
}

impl TryFrom<keystore_proto::Keystore> for LocalKeystore {
    type Error = anyhow::Error;

    fn try_from(proto: keystore_proto::Keystore) -> Result<Self> {
        Ok(Self {
            offchain_secret_key: <[u8; std::mem::size_of::<Ed25519SecretKey>()]>::try_from(
                proto.offchain_secret_key,
            )
            .map_err(|_| anyhow!("unexpectedly sized offchain_secret_key"))?,
            evm_onchain_secret_key: EcdsaSecretKey::from_slice(&proto.evm_onchain_secret_key)
                .context("invalid evm_onchain_secret_key")?,
        })
    }
}

impl From<LocalKeystore> for keystore_proto::Keystore {
    fn from(val: LocalKeystore) -> Self {
        Self {
            offchain_secret_key: val.offchain_secret_key.to_vec(),
            evm_onchain_secret_key: val.evm_onchain_secret_key.as_ref().to_vec(),
        }
    }
}

impl From<minieth::ecdsa::EthereumSignature> for EcdsaSignature {
    fn from(eth_sig: minieth::ecdsa::EthereumSignature) -> Self {
        let r = eth_sig.r.to_be_bytes();
        let s = eth_sig.s.to_be_bytes();
        let v = eth_sig.v.to_be_bytes();

        let s = match v.last().expect("least significant byte of v must exist") {
            0 => s,
            1 => {
                let order = {
                    let group = openssl::ec::EcGroup::from_curve_name(openssl::nid::Nid::SECP256K1)
                        .unwrap();
                    let mut bignum = openssl::bn::BigNum::new().unwrap();
                    let mut bignum_context = openssl::bn::BigNumContext::new().unwrap();
                    group.order(&mut bignum, &mut bignum_context).unwrap();
                    bignum
                };

                let old_bignum_s = openssl::bn::BigNum::from_slice(&s).unwrap();
                let mut new_bignum_s = openssl::bn::BigNum::new().unwrap();
                new_bignum_s.checked_sub(&order, &old_bignum_s).unwrap();
                new_bignum_s
                    .to_vec_padded(s.len() as i32)
                    .unwrap()
                    .try_into()
                    .unwrap()
            }
            v => panic!("v must be 0 or 1, not {v}"),
        };
        let r = r.into();
        let s = s.into();
        EcdsaSignature { r, s }
    }
}

impl Keystore for LocalKeystore {
    fn sign_offchain(&self, msg: &[u8]) -> Ed25519Signature {
        sign_ed25519(&self.offchain_secret_key, msg).expect("signing failed")
    }

    fn sign_evm_onchain(&self, msg: &[u8]) -> EcdsaSignature {
        let signer = minieth::ecdsa::LocalSigner::from_secret_key(self.evm_onchain_secret_key);
        let eth_sig = signer.sign_hashed(
            msg.try_into()
                .expect("input message to sign must be prehashed"),
        );
        eth_sig.into()
    }

    fn as_public(&self) -> PublicKeys {
        PublicKeys {
            offchain_public_key: {
                let skey = openssl::pkey::PKey::private_key_from_raw_bytes(
                    self.offchain_secret_key.as_ref(),
                    openssl::pkey::Id::ED25519,
                )
                .unwrap();
                skey.raw_public_key().unwrap().try_into().unwrap()
            },
            evm_onchain_public_key: minieth::ecdsa::secret_key_to_public_key(
                &self.evm_onchain_secret_key,
            )
            .into(),
        }
    }
}
