use crate::contracts::prelude::*;

pub struct DonInfo {
    pub id: u32,
    config_count: u32,
    f: u8,
    is_public: bool,
    accepts_workflows: bool,
    node_p2p_ids: Vec<Bytes32>,
    pub capability_configurations: Vec<CapabilityConfiguration>,
}

type DonInfoAsTuple = (
    u32,
    u32,
    u8,
    bool,
    bool,
    Vec<Bytes32>,
    Vec<CapabilityConfiguration>,
);

impl AbiTypeToDecode for DonInfo {
    fn abi_type_to_decode() -> Type {
        DonInfoAsTuple::abi_type_to_decode()
    }
}

impl TryFrom<Value> for DonInfo {
    type Error = AbiDecodeError;
    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let (
            id,
            config_count,
            f,
            is_public,
            accepts_workflows,
            node_p2p_ids,
            capability_configurations,
        ) = DonInfoAsTuple::try_from(value)?;
        Ok(DonInfo {
            id,
            config_count,
            f,
            is_public,
            accepts_workflows,
            node_p2p_ids,
            capability_configurations,
        })
    }
}

pub struct CapabilityConfiguration {
    pub capability_id: Bytes32,
    config: Bytes,
}

type CapabilityConfigurationAsTuple = (Bytes32, Bytes);

impl AbiTypeToDecode for CapabilityConfiguration {
    fn abi_type_to_decode() -> Type {
        CapabilityConfigurationAsTuple::abi_type_to_decode()
    }
}

impl TryFrom<Value> for CapabilityConfiguration {
    type Error = AbiDecodeError;
    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let (capability_id, config) = CapabilityConfigurationAsTuple::try_from(value)?;
        Ok(CapabilityConfiguration {
            capability_id,
            config,
        })
    }
}

pub struct GetDons;

impl ContractCall for GetDons {
    fn contract_call_signature() -> &'static str {
        "getDONs()"
    }
    fn contract_call_parameters(self) -> Value {
        ().into()
    }
}

impl ContractCallReturnValue for GetDons {
    type ReturnValue = Vec<DonInfo>;
}

#[derive(Debug, Clone)]
pub struct CapabilityId {
    pub labelled_name: String,
    pub version: String,
}

impl CapabilityId {
    pub fn hashed(&self) -> Bytes32 {
        keccak256(&(self.labelled_name.clone(), self.version.clone()).abi_encode()).into()
    }
}
