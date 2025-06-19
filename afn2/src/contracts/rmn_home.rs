use crate::config_digest::{
    empty_config_digest, prefix_config_digest, ConfigDigest, WithConfigDigest,
};
use crate::contracts::prelude::*;
use crate::ragep2p::PeerId;
use crate::signing::Ed25519PublicKey;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct U256Bitmap(U256);
impl U256Bitmap {
    pub fn contains(&self, i: usize) -> bool {
        let be = self.0.to_be_bytes();
        if i >= be.len() * 8 {
            return false;
        }
        be[be.len() - 1 - i / 8] & (1 << (i % 8)) != 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub peer_id: PeerId,
    pub offchain_public_key: Ed25519PublicKey,
}

type NodeAsTuple = (Bytes32, Bytes32);

impl AbiTypeToDecode for Node {
    fn abi_type_to_decode() -> Type {
        NodeAsTuple::abi_type_to_decode()
    }
}

impl TryFrom<Value> for Node {
    type Error = AbiDecodeError;
    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let (peer_id, offchain_public_key) = NodeAsTuple::try_from(value)?;
        Ok(Node {
            peer_id: PeerId(peer_id),
            offchain_public_key,
        })
    }
}

impl From<Node> for Value {
    fn from(val: Node) -> Self {
        (val.peer_id.0, val.offchain_public_key).into()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceChain {
    pub chain_selector: ChainSelector,
    pub f: u64,
    pub observer_nodes_bitmap: U256Bitmap,
}

type SourceChainAsTuple = (u64, u64, U256);

impl AbiTypeToDecode for SourceChain {
    fn abi_type_to_decode() -> Type {
        SourceChainAsTuple::abi_type_to_decode()
    }
}

impl TryFrom<Value> for SourceChain {
    type Error = AbiDecodeError;
    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let (chain_selector, f, observer_nodes_bitmap) = SourceChainAsTuple::try_from(value)?;
        Ok(SourceChain {
            chain_selector,
            f,
            observer_nodes_bitmap: U256Bitmap(observer_nodes_bitmap),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaticConfig {
    pub nodes: Vec<Node>,
    pub offchain_config: Bytes,
}

type StaticConfigAsTuple = (Vec<Node>, Bytes);

impl AbiTypeToDecode for StaticConfig {
    fn abi_type_to_decode() -> Type {
        StaticConfigAsTuple::abi_type_to_decode()
    }
}

impl TryFrom<Value> for StaticConfig {
    type Error = AbiDecodeError;
    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let (nodes, offchain_config) = StaticConfigAsTuple::try_from(value)?;
        Ok(StaticConfig {
            nodes,
            offchain_config,
        })
    }
}

impl From<StaticConfig> for Value {
    fn from(val: StaticConfig) -> Self {
        (val.nodes, val.offchain_config).into()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DynamicConfig {
    pub source_chains: Vec<SourceChain>,
    pub offchain_config: Bytes,
}

type DynamicConfigAsTuple = (Vec<SourceChain>, Bytes);

impl AbiTypeToDecode for DynamicConfig {
    fn abi_type_to_decode() -> Type {
        DynamicConfigAsTuple::abi_type_to_decode()
    }
}

impl TryFrom<Value> for DynamicConfig {
    type Error = AbiDecodeError;
    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let (source_chains, offchain_config) = DynamicConfigAsTuple::try_from(value)?;
        Ok(DynamicConfig {
            source_chains,
            offchain_config,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FullConfig {
    pub static_config: StaticConfig,
    pub dynamic_config: DynamicConfig,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionedConfig {
    version: u32,
    config_digest: ConfigDigest,
    static_config: StaticConfig,
    dynamic_config: DynamicConfig,
}

type VersionedConfigAsTuple = (u32, Bytes32, StaticConfig, DynamicConfig);

impl AbiTypeToDecode for VersionedConfig {
    fn abi_type_to_decode() -> Type {
        VersionedConfigAsTuple::abi_type_to_decode()
    }
}

impl TryFrom<Value> for VersionedConfig {
    type Error = AbiDecodeError;
    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let (version, config_digest, static_config, dynamic_config) =
            VersionedConfigAsTuple::try_from(value)?;
        Ok(VersionedConfig {
            version,
            config_digest,
            static_config,
            dynamic_config,
        })
    }
}

pub struct GetAllConfigs;

impl ContractCall for GetAllConfigs {
    fn contract_call_signature() -> &'static str {
        "getAllConfigs()"
    }

    fn contract_call_parameters(self) -> Value {
        ().into()
    }
}

impl ContractCallReturnValue for GetAllConfigs {
    type ReturnValue = (VersionedConfig, VersionedConfig);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckConfigDigestAuxInfo {
    pub chain_id: u64,
    pub contract: Address,
}

impl VersionedConfig {
    pub fn check_config_digest_and_convert(
        self: VersionedConfig,
        aux: CheckConfigDigestAuxInfo,
    ) -> anyhow::Result<Option<WithConfigDigest<FullConfig>>> {
        let val = self;
        let actual_config_digest = val.config_digest;
        if empty_config_digest(actual_config_digest) {
            return Ok(None);
        }

        let CheckConfigDigestAuxInfo { chain_id, contract } = aux;
        let config_digest_inputs = ConfigDigestInputs {
            chain_id,
            contract,
            version: val.version,
            static_config: val.static_config.clone(),
        };
        let expected_config_digest = config_digest(config_digest_inputs);
        if expected_config_digest != actual_config_digest {
            return Err(anyhow::anyhow!(
                "config digest mismatch, expected {:?}, got {:?}",
                expected_config_digest,
                actual_config_digest
            ));
        }

        Ok(Some(WithConfigDigest {
            config_digest: val.config_digest,
            config: FullConfig {
                static_config: val.static_config,
                dynamic_config: val.dynamic_config,
            },
        }))
    }
}

struct ConfigDigestInputs {
    chain_id: u64,
    version: u32,
    contract: Address,
    static_config: StaticConfig,
}

fn config_digest(inputs: ConfigDigestInputs) -> ConfigDigest {
    let ConfigDigestInputs {
        chain_id,
        version,
        contract,
        static_config,
    } = inputs;

    let h = keccak256(
        &[
            (
                Bytes32::from_str_literal("EVM").unwrap(),
                chain_id,
                contract,
                version,
            )
                .abi_encode()
                .as_ref(),
            (static_config,).abi_encode().as_ref(),
        ]
        .concat(),
    );

    prefix_config_digest(
        crate::config_digest::ConfigDigestPrefix::CCIPMultiRoleRMN,
        h.into(),
    )
}
