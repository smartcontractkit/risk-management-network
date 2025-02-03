use std::fmt::{Debug, Display};
use std::str::FromStr;

use anyhow::{anyhow, Context as _, Result};
use base58::{FromBase58, ToBase58};
use minieth::bytes::Bytes32;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PeerId(pub Bytes32);

const LIBP2P_PEERID_MAGIC: [u8; 6] = [0x00, 0x24, 0x08, 0x01, 0x12, 0x20];
const LIBP2P_PEERID_SIZE_WITH_MAGIC: usize =
    LIBP2P_PEERID_MAGIC.len() + std::mem::size_of::<PeerId>();

impl FromStr for PeerId {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let mut dec_with_magic = s
            .from_base58()
            .map_err(|e| anyhow!("base58 decoding failed: {e:?}"))?;
        if dec_with_magic.len() != LIBP2P_PEERID_SIZE_WITH_MAGIC {
            return Err(anyhow!(
                "unexpected length, expected {LIBP2P_PEERID_SIZE_WITH_MAGIC}, got {}",
                dec_with_magic.len()
            ));
        }
        let actual_magic = dec_with_magic.drain(..LIBP2P_PEERID_MAGIC.len());
        if actual_magic.collect::<Vec<_>>() != LIBP2P_PEERID_MAGIC {
            return Err(anyhow!("unexpected magic"));
        }
        let dec = dec_with_magic;
        Ok(PeerId(dec.try_into().context(
            "assumption violation: unexpected length even though previously checked",
        )?))
    }
}

impl Display for PeerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut enc_with_magic = [0u8; LIBP2P_PEERID_SIZE_WITH_MAGIC];
        enc_with_magic[..LIBP2P_PEERID_MAGIC.len()].copy_from_slice(&LIBP2P_PEERID_MAGIC);
        enc_with_magic[LIBP2P_PEERID_MAGIC.len()..].copy_from_slice(&self.0);
        f.write_str(&enc_with_magic.to_base58())
    }
}

impl Debug for PeerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("PeerId({})", self))
    }
}

impl PeerId {
    pub fn to_vec(self) -> Vec<u8> {
        self.0.to_vec()
    }
}

impl TryFrom<Vec<u8>> for PeerId {
    type Error = anyhow::Error;

    fn try_from(bytes: Vec<u8>) -> Result<Self> {
        Ok(PeerId(bytes.try_into()?))
    }
}

#[derive(Clone, Debug)]
pub struct Bootstrapper {
    pub peer_id: PeerId,
    pub addrs: Vec<String>,
}

impl FromStr for Bootstrapper {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let mut parts = s.split('@');
        let peer_id = parts.next().context("missing peer id")?;
        let peer_id = PeerId::from_str(peer_id)?;
        let addrs = parts.next().context("missing host:ports")?.split('/');
        if !parts.next().is_none() {
            return Err(anyhow!("unexpected multiple @ symbols"));
        }
        let addrs = addrs
            .into_iter()
            .map(|host_port| {
                let mut parts = host_port.split(':');
                parts.next().context("missing host")?;
                let port = parts
                    .next()
                    .context("missing port")?
                    .parse::<u16>()
                    .context("invalid port")?;
                if !(port > 0) {
                    return Err(anyhow!("port must be positive"));
                }
                if !parts.next().is_none() {
                    return Err(anyhow!("unexpected multiple : symbols"));
                }
                Ok(host_port.to_string())
            })
            .collect::<Result<Vec<String>>>()?;
        Ok(Self { peer_id, addrs })
    }
}

impl<'de> serde::Deserialize<'de> for Bootstrapper {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Bootstrapper::from_str(&s).map_err(serde::de::Error::custom)
    }
}
