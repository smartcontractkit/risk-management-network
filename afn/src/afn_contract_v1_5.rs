use crate::{
    chain_selector::chain_selector,
    common::{ChainName, ContractCall},
    curse_beacon::CurseId,
};
use anyhow::{anyhow, Result};
use miniabi::{
    abi_decode::{AbiDecodeError, AbiTypeToDecode},
    types::{Type, Value},
};
use minieth::bytes::{Address, Bytes16};
use std::collections::HashSet;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct VoterV1_5 {
    pub bless_vote_addr: Address,
    pub curse_vote_addr: Address,
    pub bless_weight: u8,
    pub curse_weight: u8,
}
type VoterV1_5AsTuple = (Address, Address, u8, u8);
impl AbiTypeToDecode for VoterV1_5 {
    fn abi_type_to_decode() -> Type {
        VoterV1_5AsTuple::abi_type_to_decode()
    }
}
impl TryFrom<Value> for VoterV1_5 {
    type Error = AbiDecodeError;
    fn try_from(v: Value) -> Result<Self, Self::Error> {
        let (bless_vote_addr, curse_vote_addr, bless_weight, curse_weight) =
            VoterV1_5AsTuple::try_from(v)?;
        Ok(Self {
            bless_vote_addr,
            curse_vote_addr,
            bless_weight,
            curse_weight,
        })
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct OnchainConfigV1_5 {
    pub voters: Vec<VoterV1_5>,
    pub bless_weight_threshold: u16,
    pub curse_weight_threshold: u16,
}

type OnchainConfigV1_5AsTuple = (Vec<VoterV1_5>, u16, u16);

impl AbiTypeToDecode for OnchainConfigV1_5 {
    fn abi_type_to_decode() -> Type {
        OnchainConfigV1_5AsTuple::abi_type_to_decode()
    }
}

impl TryFrom<Value> for OnchainConfigV1_5 {
    type Error = AbiDecodeError;
    fn try_from(value: Value) -> std::result::Result<Self, Self::Error> {
        let (voters, bless_weight_threshold, curse_weight_threshold) =
            OnchainConfigV1_5AsTuple::try_from(value)?;
        Ok(Self {
            voters,
            bless_weight_threshold,
            curse_weight_threshold,
        })
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct VoteToCurseCallV1_5 {
    pub curse_id: Bytes16,
    pub subjects: Vec<Bytes16>,
}
impl ContractCall for VoteToCurseCallV1_5 {
    fn contract_call_signature() -> &'static str {
        "voteToCurse(bytes16,bytes16[])"
    }
    fn contract_call_parameters(self) -> Value {
        (self.curse_id, self.subjects).into()
    }
}

pub const GLOBAL_CURSE_SUBJECT: [u8; 16] = [
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
];
pub const LEGACY_CURSE_SUBJECT: [u8; 16] = [
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

#[derive(Debug, Clone, Eq, Hash, PartialEq)]
pub enum CurseSubjectV1_5 {
    Chain(ChainName),
    GlobalCurseSubject,
    LegacyCurseSubject,
}
impl CurseSubjectV1_5 {
    fn to_bytes16_curse_subject(&self) -> Bytes16 {
        match self {
            CurseSubjectV1_5::Chain(chain_name) => {
                Bytes16::from(u128::from(chain_selector(*chain_name)).to_be_bytes())
            }
            CurseSubjectV1_5::GlobalCurseSubject => Bytes16::from(GLOBAL_CURSE_SUBJECT),
            CurseSubjectV1_5::LegacyCurseSubject => Bytes16::from(LEGACY_CURSE_SUBJECT),
        }
    }
}

pub fn get_vote_to_curse_call_and_gas_limit_v1_5(
    curse_id: CurseId,
    curse_subjects: &HashSet<CurseSubjectV1_5>,
) -> Result<(VoteToCurseCallV1_5, u64)> {
    let sorted_subjects: Vec<Bytes16> = {
        let mut subjects: Vec<Bytes16> = curse_subjects
            .iter()
            .map(CurseSubjectV1_5::to_bytes16_curse_subject)
            .collect();
        subjects.sort_by(|a, b| {
            let a_as_u128 = u128::from_be_bytes((*a).into());
            let b_as_u128 = u128::from_be_bytes((*b).into());
            a_as_u128.cmp(&b_as_u128)
        });
        subjects
    };
    let num_of_subjects = sorted_subjects.len();
    let gas_limit = 200_000u64
        .checked_mul(u64::try_from(num_of_subjects)?)
        .ok_or(anyhow!("gas limit can't fit into u64"))?;
    Ok((
        VoteToCurseCallV1_5 {
            curse_id,
            subjects: sorted_subjects,
        },
        gas_limit,
    ))
}
