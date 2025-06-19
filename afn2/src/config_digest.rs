use minieth::bytes::Bytes32;

pub type ConfigDigest = Bytes32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WithConfigDigest<T> {
    pub config_digest: ConfigDigest,
    pub config: T,
}

pub fn empty_config_digest(config_digest: ConfigDigest) -> bool {
    config_digest == [0; 32].into()
}

#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConfigDigestPrefix {
    CCIPMultiRole = 0x000a,
    CCIPMultiRoleRMN = 0x000b,
    CCIPMultiRoleRMNCombo = 0x000c,
}

pub fn prefix_config_digest(prefix: ConfigDigestPrefix, digest: ConfigDigest) -> ConfigDigest {
    let prefix = (prefix as u16).to_be_bytes();
    let mut digest: [u8; 32] = digest.into();
    digest[..prefix.len()].copy_from_slice(&prefix);
    digest.into()
}

pub fn check_config_digest_prefix(prefix: ConfigDigestPrefix, digest: ConfigDigest) -> bool {
    let prefix = (prefix as u16).to_be_bytes();
    let digest: [u8; 32] = digest.into();
    digest[..prefix.len()] == prefix
}
