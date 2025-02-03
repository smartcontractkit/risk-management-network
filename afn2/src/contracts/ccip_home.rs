use std::time::Duration;

use anyhow::Context;
use serde::Deserialize;

use crate::{
    config_digest::{empty_config_digest, prefix_config_digest, ConfigDigest, WithConfigDigest},
    contracts::prelude::*,
    offchainreporting3_config::{OffchainConfigProto, OCR3_OFFCHAIN_CONFIG_VERSION},
    ragep2p::PeerId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OcrPluginType {
    Commit,
    Execution,
}

impl AbiTypeToDecode for OcrPluginType {
    fn abi_type_to_decode() -> Type {
        Type::U8
    }
}

impl TryFrom<Value> for OcrPluginType {
    type Error = AbiDecodeError;
    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let v: u8 = value.try_into()?;
        match v {
            0 => Ok(OcrPluginType::Commit),
            1 => Ok(OcrPluginType::Execution),
            _ => Err(AbiDecodeError::InvalidValueError),
        }
    }
}

impl From<OcrPluginType> for Value {
    fn from(val: OcrPluginType) -> Self {
        (val as u8).into()
    }
}

#[derive(Debug, Clone)]
pub struct Ocr3Node {
    pub p2p_id: Bytes32,
    signer_key: Bytes,
    transmitter_key: Bytes,
}

type Ocr3NodeAsTuple = (Bytes32, Bytes, Bytes);

impl AbiTypeToDecode for Ocr3Node {
    fn abi_type_to_decode() -> Type {
        Ocr3NodeAsTuple::abi_type_to_decode()
    }
}

impl TryFrom<Value> for Ocr3Node {
    type Error = AbiDecodeError;
    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let (p2p_id, signer_key, transmitter_key) = Ocr3NodeAsTuple::try_from(value)?;
        Ok(Ocr3Node {
            p2p_id,
            signer_key,
            transmitter_key,
        })
    }
}

impl From<Ocr3Node> for Value {
    fn from(val: Ocr3Node) -> Self {
        (val.p2p_id, val.signer_key, val.transmitter_key).into()
    }
}

#[derive(Debug, Clone)]
pub struct Ocr3Config {
    plugin_type: OcrPluginType,
    pub chain_selector: u64,
    f: u8,
    offchain_config_version: u64,
    offramp_address: Bytes,
    rmn_home_address: Bytes,
    pub nodes: Vec<Ocr3Node>,
    offchain_config: Bytes,
}

type Ocr3ConfigAsTuple = (
    OcrPluginType,
    u64,
    u8,
    u64,
    Bytes,
    Bytes,
    Vec<Ocr3Node>,
    Bytes,
);

impl AbiTypeToDecode for Ocr3Config {
    fn abi_type_to_decode() -> Type {
        Ocr3ConfigAsTuple::abi_type_to_decode()
    }
}

impl TryFrom<Value> for Ocr3Config {
    type Error = AbiDecodeError;
    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let (
            plugin_type,
            chain_selector,
            f,
            offchain_config_version,
            offramp_address,
            rmn_home_address,
            nodes,
            offchain_config,
        ) = Ocr3ConfigAsTuple::try_from(value)?;
        Ok(Ocr3Config {
            plugin_type,
            chain_selector,
            f,
            offchain_config_version,
            offramp_address,
            rmn_home_address,
            nodes,
            offchain_config,
        })
    }
}

impl From<Ocr3Config> for Value {
    fn from(val: Ocr3Config) -> Self {
        (
            val.plugin_type,
            val.chain_selector,
            val.f,
            val.offchain_config_version,
            val.offramp_address,
            val.rmn_home_address,
            val.nodes,
            val.offchain_config,
        )
            .into()
    }
}

struct ConfigDigestInputs {
    chain_id: u64,
    contract: Address,
    don_id: u32,
    plugin_type: OcrPluginType,
    version: u32,
    ocr3_config: Ocr3Config,
}

fn config_digest(inputs: ConfigDigestInputs) -> ConfigDigest {
    let ConfigDigestInputs {
        chain_id,
        contract,
        don_id,
        plugin_type,
        version,
        ocr3_config,
    } = inputs;

    let h = keccak256(
        &[
            (
                Bytes32::from_str_literal("EVM").unwrap(),
                chain_id,
                contract,
                don_id,
                plugin_type,
                version,
            )
                .abi_encode()
                .as_ref(),
            (ocr3_config,).abi_encode().as_ref(),
        ]
        .concat(),
    );

    prefix_config_digest(
        crate::config_digest::ConfigDigestPrefix::CCIPMultiRole,
        h.into(),
    )
}

#[derive(Debug, Clone)]
pub struct VersionedConfig {
    pub version: u32,
    pub config_digest: Bytes32,
    pub config: Ocr3Config,
}

type VersionedConfigAsTuple = (u32, Bytes32, Ocr3Config);

impl AbiTypeToDecode for VersionedConfig {
    fn abi_type_to_decode() -> Type {
        VersionedConfigAsTuple::abi_type_to_decode()
    }
}

impl TryFrom<Value> for VersionedConfig {
    type Error = AbiDecodeError;
    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let (version, config_digest, config) = VersionedConfigAsTuple::try_from(value)?;
        Ok(VersionedConfig {
            version,
            config_digest,
            config,
        })
    }
}

#[derive(Debug, Clone)]
pub struct GetAllConfigs {
    pub don_id: u32,
    pub plugin_type: OcrPluginType,
}

impl ContractCall for GetAllConfigs {
    fn contract_call_signature() -> &'static str {
        "getAllConfigs(uint32,uint8)"
    }
    fn contract_call_parameters(self) -> Value {
        (self.don_id, self.plugin_type as u8).into()
    }
}

impl ContractCallReturnValue for GetAllConfigs {
    type ReturnValue = (VersionedConfig, VersionedConfig);
}

#[derive(Debug, Copy, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommitPluginOffchainConfig {
    pub rmn_enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CcipOcrConfig {
    pub dest_chain_selector: ChainSelector,
    pub peer_ids: Vec<PeerId>,
    pub estimated_round_interval: Duration,
    pub rmn_enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckConfigDigestAuxInfo {
    pub chain_id: u64,
    pub contract: Address,
    pub don_id: u32,
}

impl VersionedConfig {
    pub fn check_config_digest_and_convert(
        self: VersionedConfig,
        aux: CheckConfigDigestAuxInfo,
    ) -> anyhow::Result<Option<WithConfigDigest<CcipOcrConfig>>> {
        let val = self;
        let actual_config_digest = val.config_digest;
        if empty_config_digest(actual_config_digest) {
            return Ok(None);
        }
        let config_digest_inputs = ConfigDigestInputs {
            chain_id: aux.chain_id,
            contract: aux.contract,
            don_id: aux.don_id,
            plugin_type: OcrPluginType::Commit,
            version: val.version,
            ocr3_config: val.config.clone(),
        };
        let expected_config_digest = config_digest(config_digest_inputs);
        if expected_config_digest != actual_config_digest {
            return Err(anyhow::anyhow!(
                "config digest mismatch, expected {:?}, got {:?}",
                expected_config_digest,
                actual_config_digest
            ));
        }

        let decoded_offchain_config = match val.config.offchain_config_version {
            OCR3_OFFCHAIN_CONFIG_VERSION => {
                OffchainConfigProto::decode(&val.config.offchain_config)
                    .context("while decoding OffchainConfigProto")?
            }
            unsupported_version => {
                return Err(anyhow::anyhow!(
                    "unsupported offchain config version: {}",
                    unsupported_version
                ))
            }
        };

        let rmn_enabled = serde_json::from_slice::<CommitPluginOffchainConfig>(
            &decoded_offchain_config.reporting_plugin_config,
        )
        .context("while deserializing CommitPluginOffchainConfig")?
        .rmn_enabled;

        Ok(Some(WithConfigDigest {
            config_digest: val.config_digest,
            config: CcipOcrConfig {
                dest_chain_selector: val.config.chain_selector,
                peer_ids: val
                    .config
                    .nodes
                    .into_iter()
                    .map(|n| PeerId(n.p2p_id))
                    .collect(),
                estimated_round_interval: Duration::from_nanos(
                    decoded_offchain_config.delta_round_nanoseconds,
                ),
                rmn_enabled,
            },
        }))
    }
}
