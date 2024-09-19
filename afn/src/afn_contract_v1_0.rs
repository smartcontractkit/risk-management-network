use crate::{common::ContractCall, curse_beacon::CurseId};
use anyhow::Result;
use miniabi::{
    abi_decode::{AbiDecodeError, AbiTypeToDecode},
    types::{Type, Value},
};
use minieth::bytes::{Address, Bytes32};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct VoterV1_0 {
    pub bless_vote_addr: Address,
    pub curse_vote_addr: Address,
    pub curse_unvote_addr: Address,
    pub bless_weight: u8,
    pub curse_weight: u8,
}
type VoterV1_0AsTuple = (Address, Address, Address, u8, u8);
impl AbiTypeToDecode for VoterV1_0 {
    fn abi_type_to_decode() -> Type {
        VoterV1_0AsTuple::abi_type_to_decode()
    }
}
impl TryFrom<Value> for VoterV1_0 {
    type Error = AbiDecodeError;
    fn try_from(v: Value) -> Result<Self, Self::Error> {
        let (bless_vote_addr, curse_vote_addr, curse_unvote_addr, bless_weight, curse_weight) =
            VoterV1_0AsTuple::try_from(v)?;
        Ok(Self {
            bless_vote_addr,
            curse_vote_addr,
            curse_unvote_addr,
            bless_weight,
            curse_weight,
        })
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct OnchainConfigV1_0 {
    pub voters: Vec<VoterV1_0>,
    pub bless_weight_threshold: u16,
    pub curse_weight_threshold: u16,
}

type OnchainConfigV1_0AsTuple = (Vec<VoterV1_0>, u16, u16);

impl AbiTypeToDecode for OnchainConfigV1_0 {
    fn abi_type_to_decode() -> Type {
        OnchainConfigV1_0AsTuple::abi_type_to_decode()
    }
}

impl TryFrom<Value> for OnchainConfigV1_0 {
    type Error = AbiDecodeError;
    fn try_from(value: Value) -> std::result::Result<Self, Self::Error> {
        let (voters, bless_weight_threshold, curse_weight_threshold) =
            OnchainConfigV1_0AsTuple::try_from(value)?;
        Ok(Self {
            voters,
            bless_weight_threshold,
            curse_weight_threshold,
        })
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct VoteToCurseCallV1_0 {
    pub curse_id: Bytes32,
}
impl ContractCall for VoteToCurseCallV1_0 {
    fn contract_call_signature() -> &'static str {
        "voteToCurse(bytes32)"
    }
    fn contract_call_parameters(self) -> Value {
        (self.curse_id,).into()
    }
}

pub fn get_vote_to_curse_call_and_gas_limit_v1_0(curse_id: CurseId) -> (VoteToCurseCallV1_0, u64) {
    (
        VoteToCurseCallV1_0 {
            curse_id: Bytes32::from_vec([curse_id.into(), [0u8; 16]].concat()).unwrap(),
        },
        200_000u64,
    )
}
