use anyhow::{bail, Context, Result};
use minieth::bytes::{Bytes, SizedBytes};
use openssl;
use serde::{Deserialize, Serialize};

type Iv = SizedBytes<32>;

const TAG_SIZE_IN_BYTES: usize = 16;
type Tag = SizedBytes<TAG_SIZE_IN_BYTES>;

type Salt = SizedBytes<32>;

const SCRYPT_N: u64 = 262144;

const SCRYPT_P: u64 = 1;

const SCRYPT_R: u64 = 8;

const SCRYPT_DKLEN: usize = 16;

const SCRYPT_MAXMEM: u64 = (SCRYPT_P * 128 * SCRYPT_R) + 32 * SCRYPT_R * (SCRYPT_N + 2) * 4;

const EXPECTED_VERSION: u64 = 1;

fn derive_key(salt: Salt, passphrase: &str) -> Result<[u8; SCRYPT_DKLEN]> {
    let mut derived_key = [0u8; SCRYPT_DKLEN];
    openssl::pkcs5::scrypt(
        passphrase.as_bytes(),
        salt.as_ref(),
        SCRYPT_N,
        SCRYPT_R,
        SCRYPT_P,
        SCRYPT_MAXMEM,
        &mut derived_key,
    )?;
    Ok(derived_key)
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct EncryptedSecrets {
    iv: Iv,
    ciphertext: Bytes,
    tag: Tag,
    associated_data: String,
    scrypt_salt: Salt,
    version: u64,
}

impl EncryptedSecrets {
    pub fn decrypt(&self, passphrase: &str) -> Result<Vec<u8>> {
        if self.version != EXPECTED_VERSION {
            bail!(
                "unsupported EncryptedSecrets version, expected {}, got {}",
                EXPECTED_VERSION,
                self.version
            );
        }
        let derived_key = derive_key(self.scrypt_salt, passphrase)?;
        let plaintext = openssl::symm::decrypt_aead(
            openssl::symm::Cipher::aes_128_gcm(),
            &derived_key,
            Some(self.iv.as_ref()),
            self.associated_data.as_ref(),
            self.ciphertext.as_ref(),
            self.tag.as_ref(),
        )
        .context("openssl failed to decrypt EncryptedSecrets")?;
        Ok(plaintext)
    }

    fn random_bytes<const N: usize>() -> Result<[u8; N]> {
        let mut b = [0u8; N];
        openssl::rand::rand_bytes(&mut b)?;
        Ok(b)
    }

    fn encrypt_internal(
        plaintext: &[u8],
        passphrase: &str,
        salt: Salt,
        iv: Iv,
        aad: &str,
    ) -> Result<EncryptedSecrets> {
        let version = EXPECTED_VERSION;
        let derived_key = derive_key(salt, passphrase)?;
        let mut tag = [0u8; TAG_SIZE_IN_BYTES];
        let ciphertext = openssl::symm::encrypt_aead(
            openssl::symm::Cipher::aes_128_gcm(),
            &derived_key,
            Some(iv.as_ref()),
            aad.as_bytes(),
            plaintext,
            &mut tag,
        )
        .context("openssl failed to encrypt EncryptedSecrets")?;
        Ok(EncryptedSecrets {
            iv,
            ciphertext: Bytes::from(ciphertext),
            associated_data: aad.to_string(),
            tag: tag.into(),
            scrypt_salt: salt,
            version,
        })
    }

    pub fn encrypt(
        plaintext: &[u8],
        passphrase: &str,
        associated_data: &str,
    ) -> Result<EncryptedSecrets> {
        Self::encrypt_internal(
            plaintext,
            passphrase,
            Self::random_bytes()?.into(),
            Self::random_bytes()?.into(),
            associated_data,
        )
    }
}
