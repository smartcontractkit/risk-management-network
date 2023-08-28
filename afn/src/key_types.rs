use crate::{common::ChainName, encryption, encryption::EncryptedSecrets};
use anyhow::Result;
use minieth::bytes::{Address, Bytes32};
use minieth::ecdsa::{generate_secret_key, secret_key_to_public_key};
use minieth::{bytes::SizedBytes, ecdsa::LocalSigner};
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;
use std::fmt::{Debug, Display};

const EVM_SECRET_KEY_SIZE: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SecretKey(minieth::ecdsa::SecretKey);

impl SecretKey {
    pub fn generate() -> Self {
        Self(generate_secret_key())
    }

    pub fn local_signer(&self) -> LocalSigner {
        LocalSigner::from_secret_key(self.0)
    }

    pub fn address(&self) -> Address {
        secret_key_to_public_key(&self.0).into()
    }
}

impl Serialize for SecretKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let b32: Bytes32 = self.0.secret_bytes().into();
        b32.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SecretKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let b32 = Bytes32::deserialize(deserializer)?;
        let secret_key =
            minieth::ecdsa::SecretKey::from_slice(b32.as_ref()).map_err(de::Error::custom)?;
        Ok(Self(secret_key))
    }
}

impl TryFrom<SizedBytes<EVM_SECRET_KEY_SIZE>> for SecretKey {
    type Error = anyhow::Error;
    fn try_from(value: SizedBytes<EVM_SECRET_KEY_SIZE>) -> std::result::Result<Self, Self::Error> {
        let secret_key = minieth::ecdsa::SecretKey::from_slice(value.as_ref())?;
        Ok(Self(secret_key))
    }
}

impl Debug for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.address())
    }
}

impl Display for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.address())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlessCurseKeys {
    pub bless: SecretKey,
    pub curse: SecretKey,
}

impl BlessCurseKeys {
    pub fn generate() -> BlessCurseKeys {
        BlessCurseKeys {
            bless: SecretKey::generate(),
            curse: SecretKey::generate(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlessCurseKeysByChain(pub HashMap<ChainName, BlessCurseKeys>);

impl BlessCurseKeysByChain {
    pub fn generate(chains: &[ChainName]) -> BlessCurseKeysByChain {
        let mut by_chain = HashMap::new();
        for &chain in chains {
            by_chain.insert(chain, BlessCurseKeys::generate());
        }
        BlessCurseKeysByChain(by_chain)
    }

    pub fn get(&self, chain_name: ChainName) -> Option<&BlessCurseKeys> {
        self.0.get(&chain_name)
    }

    pub fn insert(
        &mut self,
        chain_name: ChainName,
        bless_curse_keys: BlessCurseKeys,
    ) -> Option<BlessCurseKeys> {
        self.0.insert(chain_name, bless_curse_keys)
    }

    pub fn chains(&self) -> Vec<ChainName> {
        self.0.keys().cloned().collect()
    }

    pub fn from_reader(reader: impl std::io::Read, passphrase: &str) -> Result<Self> {
        let encrypted: EncryptedSecrets = serde_json::from_reader(reader)?;
        Self::decrypt(&encrypted, passphrase)
    }

    pub fn encrypt(&self, passphrase: &str) -> Result<EncryptedSecrets> {
        let plaintext = serde_json::to_vec(&self)?;
        let aad = format!("{:?}", self);
        encryption::EncryptedSecrets::encrypt(&plaintext, passphrase, &aad)
    }

    pub fn decrypt(keystore: &EncryptedSecrets, passphrase: &str) -> Result<Self> {
        let data = keystore.decrypt(passphrase)?;
        Ok(serde_json::from_slice(&data)?)
    }
}
