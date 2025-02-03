use crate::config_digest::ConfigDigest;
use crate::offchain_proto as offchain;
use crate::signing::{EcdsaSignature, Ed25519Signature};

use afn::chains::ChainSelector;
use minieth::bytes::{Address, Bytes32};

use anyhow::{anyhow, Context as _, Result};
use prost::Message;

impl TryFrom<offchain::EcdsaSignature> for EcdsaSignature {
    type Error = anyhow::Error;

    fn try_from(proto: offchain::EcdsaSignature) -> std::result::Result<Self, Self::Error> {
        Ok(EcdsaSignature {
            r: proto.r.try_into()?,
            s: proto.s.try_into()?,
        })
    }
}

impl From<EcdsaSignature> for offchain::EcdsaSignature {
    fn from(val: EcdsaSignature) -> Self {
        offchain::EcdsaSignature {
            r: val.r.to_vec(),
            s: val.s.to_vec(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct LaneDest {
    pub dest_chain_selector: ChainSelector,
    pub offramp_address: Address,
}

impl TryFrom<offchain::LaneDest> for LaneDest {
    type Error = anyhow::Error;

    fn try_from(proto: offchain::LaneDest) -> std::result::Result<Self, Self::Error> {
        Ok(LaneDest {
            dest_chain_selector: proto.dest_chain_selector,
            offramp_address: proto.offramp_address.try_into()?,
        })
    }
}

impl From<LaneDest> for offchain::LaneDest {
    fn from(val: LaneDest) -> Self {
        offchain::LaneDest {
            dest_chain_selector: val.dest_chain_selector,
            offramp_address: val.offramp_address.to_vec(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct LaneSource {
    pub source_chain_selector: ChainSelector,
    pub onramp_address: Address,
}

impl TryFrom<offchain::LaneSource> for LaneSource {
    type Error = anyhow::Error;

    fn try_from(proto: offchain::LaneSource) -> std::result::Result<Self, Self::Error> {
        Ok(LaneSource {
            source_chain_selector: proto.source_chain_selector,
            onramp_address: proto.onramp_address.try_into()?,
        })
    }
}

impl From<LaneSource> for offchain::LaneSource {
    fn from(val: LaneSource) -> Self {
        offchain::LaneSource {
            source_chain_selector: val.source_chain_selector,
            onramp_address: val.onramp_address.to_vec(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ClosedInterval {
    pub min_msg_nr: u64,
    pub max_msg_nr: u64,
}

impl TryFrom<offchain::ClosedInterval> for ClosedInterval {
    type Error = anyhow::Error;

    fn try_from(proto: offchain::ClosedInterval) -> std::result::Result<Self, Self::Error> {
        Ok(ClosedInterval {
            min_msg_nr: proto.min_msg_nr,
            max_msg_nr: proto.max_msg_nr,
        })
    }
}

impl From<ClosedInterval> for offchain::ClosedInterval {
    fn from(val: ClosedInterval) -> Self {
        offchain::ClosedInterval {
            min_msg_nr: val.min_msg_nr,
            max_msg_nr: val.max_msg_nr,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FixedDestLaneUpdateRequest {
    pub lane_source: LaneSource,
    pub closed_interval: ClosedInterval,
}

impl TryFrom<offchain::FixedDestLaneUpdateRequest> for FixedDestLaneUpdateRequest {
    type Error = anyhow::Error;

    fn try_from(
        proto: offchain::FixedDestLaneUpdateRequest,
    ) -> std::result::Result<Self, Self::Error> {
        Ok(FixedDestLaneUpdateRequest {
            lane_source: proto
                .lane_source
                .context("lane_source missing")?
                .try_into()?,
            closed_interval: proto
                .closed_interval
                .context("closed_interval missing")?
                .try_into()?,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct FixedDestLaneUpdate {
    pub lane_source: LaneSource,
    pub closed_interval: ClosedInterval,
    pub root: Bytes32,
}

impl TryFrom<offchain::FixedDestLaneUpdate> for FixedDestLaneUpdate {
    type Error = anyhow::Error;

    fn try_from(proto: offchain::FixedDestLaneUpdate) -> std::result::Result<Self, Self::Error> {
        Ok(FixedDestLaneUpdate {
            lane_source: proto
                .lane_source
                .context("lane_source missing")?
                .try_into()?,
            closed_interval: proto
                .closed_interval
                .context("closed_interval missing")?
                .try_into()?,
            root: proto.root.try_into()?,
        })
    }
}

impl From<FixedDestLaneUpdate> for offchain::FixedDestLaneUpdate {
    fn from(val: FixedDestLaneUpdate) -> Self {
        offchain::FixedDestLaneUpdate {
            lane_source: Some(val.lane_source.into()),
            closed_interval: Some(val.closed_interval.into()),
            root: val.root.to_vec(),
        }
    }
}

#[derive(Debug)]
pub struct ObservationRequest {
    pub lane_dest: LaneDest,
    pub fixed_dest_lane_update_requests: Vec<FixedDestLaneUpdateRequest>,
}

impl TryFrom<offchain::ObservationRequest> for ObservationRequest {
    type Error = anyhow::Error;

    fn try_from(proto: offchain::ObservationRequest) -> std::result::Result<Self, Self::Error> {
        Ok(ObservationRequest {
            lane_dest: proto.lane_dest.context("lane_dest missing")?.try_into()?,
            fixed_dest_lane_update_requests: proto
                .fixed_dest_lane_update_requests
                .into_iter()
                .map(|proto| proto.try_into())
                .collect::<Result<_>>()?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Observation {
    pub rmn_home_contract_config_digest: Bytes32,
    pub lane_dest: LaneDest,
    pub fixed_dest_lane_updates: Vec<FixedDestLaneUpdate>,
    pub timestamp: u64,
}

impl TryFrom<offchain::Observation> for Observation {
    type Error = anyhow::Error;

    fn try_from(proto: offchain::Observation) -> std::result::Result<Self, Self::Error> {
        Ok(Observation {
            rmn_home_contract_config_digest: proto.rmn_home_contract_config_digest.try_into()?,
            lane_dest: proto.lane_dest.context("lane_dest missing")?.try_into()?,
            fixed_dest_lane_updates: proto
                .fixed_dest_lane_updates
                .into_iter()
                .map(|proto| proto.try_into())
                .collect::<Result<_>>()?,
            timestamp: proto.timestamp,
        })
    }
}

impl From<Observation> for offchain::Observation {
    fn from(val: Observation) -> Self {
        offchain::Observation {
            rmn_home_contract_config_digest: val.rmn_home_contract_config_digest.to_vec(),
            lane_dest: Some(val.lane_dest.into()),
            fixed_dest_lane_updates: val
                .fixed_dest_lane_updates
                .into_iter()
                .map(|proto| proto.into())
                .collect(),
            timestamp: val.timestamp,
        }
    }
}

impl Observation {
    pub fn encode_to_vec(self: Observation) -> Vec<u8> {
        offchain::Observation::from(self).encode_to_vec()
    }
}

#[derive(Debug)]
pub struct ReportContext {
    pub evm_dest_chain_id: u64,
    pub rmn_remote_contract_address: Address,
    pub rmn_home_contract_config_digest: Bytes32,
    pub lane_dest: LaneDest,
}

impl TryFrom<offchain::ReportContext> for ReportContext {
    type Error = anyhow::Error;

    fn try_from(proto: offchain::ReportContext) -> std::result::Result<Self, Self::Error> {
        Ok(ReportContext {
            evm_dest_chain_id: proto.evm_dest_chain_id,
            rmn_remote_contract_address: proto.rmn_remote_contract_address.try_into()?,
            rmn_home_contract_config_digest: proto.rmn_home_contract_config_digest.try_into()?,
            lane_dest: proto.lane_dest.context("lane_dest missing")?.try_into()?,
        })
    }
}

#[derive(Debug)]
pub struct SignedObservation {
    pub observation: Observation,
    pub signature: Ed25519Signature,
}

impl TryFrom<offchain::SignedObservation> for SignedObservation {
    type Error = anyhow::Error;

    fn try_from(proto: offchain::SignedObservation) -> std::result::Result<Self, Self::Error> {
        Ok(SignedObservation {
            observation: proto
                .observation
                .context("observation missing")?
                .try_into()?,
            signature: proto
                .signature
                .try_into()
                .map_err(|_| anyhow!("signature of unexpected size"))?,
        })
    }
}

impl From<SignedObservation> for offchain::SignedObservation {
    fn from(val: SignedObservation) -> Self {
        offchain::SignedObservation {
            observation: Some(val.observation.into()),
            signature: val.signature.to_vec(),
        }
    }
}

#[derive(Debug)]
pub struct AttributedSignedObservation {
    pub signed_observation: SignedObservation,
    pub signer_node_index: u32,
}

impl TryFrom<offchain::AttributedSignedObservation> for AttributedSignedObservation {
    type Error = anyhow::Error;

    fn try_from(
        proto: offchain::AttributedSignedObservation,
    ) -> std::result::Result<Self, Self::Error> {
        Ok(AttributedSignedObservation {
            signed_observation: proto
                .signed_observation
                .context("signed_observation missing")?
                .try_into()?,
            signer_node_index: proto.signer_node_index,
        })
    }
}

#[derive(Debug)]
pub struct ReportSignatureRequest {
    pub context: ReportContext,
    pub attributed_signed_observations: Vec<AttributedSignedObservation>,
}

impl TryFrom<offchain::ReportSignatureRequest> for ReportSignatureRequest {
    type Error = anyhow::Error;

    fn try_from(proto: offchain::ReportSignatureRequest) -> std::result::Result<Self, Self::Error> {
        Ok(ReportSignatureRequest {
            context: proto.context.context("context missing")?.try_into()?,
            attributed_signed_observations: proto
                .attributed_signed_observations
                .into_iter()
                .map(|proto| proto.try_into())
                .collect::<Result<_>>()?,
        })
    }
}

#[derive(Debug)]
pub struct ReportSignature {
    pub signature: EcdsaSignature,
}

impl TryFrom<offchain::ReportSignature> for ReportSignature {
    type Error = anyhow::Error;

    fn try_from(proto: offchain::ReportSignature) -> std::result::Result<Self, Self::Error> {
        Ok(ReportSignature {
            signature: proto.signature.context("signature missing")?.try_into()?,
        })
    }
}

impl From<ReportSignature> for offchain::ReportSignature {
    fn from(val: ReportSignature) -> Self {
        offchain::ReportSignature {
            signature: Some(val.signature.into()),
        }
    }
}

#[derive(Debug)]
pub enum Request {
    ObservationRequest(ObservationRequest),
    ReportSignatureRequest(ReportSignatureRequest),
}

impl TryFrom<offchain::request::Request> for Request {
    type Error = anyhow::Error;

    fn try_from(proto: offchain::request::Request) -> std::result::Result<Self, Self::Error> {
        match proto {
            offchain::request::Request::ObservationRequest(proto) => {
                Ok(Request::ObservationRequest(proto.try_into()?))
            }
            offchain::request::Request::ReportSignatureRequest(proto) => {
                Ok(Request::ReportSignatureRequest(proto.try_into()?))
            }
        }
    }
}

#[derive(Debug)]
pub struct RequestWithId {
    pub request_id: u64,
    pub request: Request,
}

impl TryFrom<offchain::Request> for RequestWithId {
    type Error = anyhow::Error;

    fn try_from(proto: offchain::Request) -> std::result::Result<Self, Self::Error> {
        Ok(RequestWithId {
            request_id: proto.request_id,
            request: proto.request.context("request missing")?.try_into()?,
        })
    }
}

impl RequestWithId {
    pub fn decode(slice: &[u8]) -> anyhow::Result<Self> {
        let proto = offchain::Request::decode(slice)?;
        proto.try_into()
    }
}

#[derive(Debug)]
pub enum Response {
    SignedObservation(SignedObservation),
    ReportSignature(ReportSignature),
}

impl TryFrom<offchain::response::Response> for Response {
    type Error = anyhow::Error;

    fn try_from(proto: offchain::response::Response) -> std::result::Result<Self, Self::Error> {
        match proto {
            offchain::response::Response::SignedObservation(proto) => {
                Ok(Response::SignedObservation(proto.try_into()?))
            }
            offchain::response::Response::ReportSignature(proto) => {
                Ok(Response::ReportSignature(proto.try_into()?))
            }
        }
    }
}

impl From<Response> for offchain::response::Response {
    fn from(val: Response) -> Self {
        match val {
            Response::SignedObservation(proto) => {
                offchain::response::Response::SignedObservation(proto.into())
            }
            Response::ReportSignature(proto) => {
                offchain::response::Response::ReportSignature(proto.into())
            }
        }
    }
}

pub struct ResponseWithId {
    pub request_id: u64,
    pub response: Response,
}

impl TryFrom<offchain::Response> for ResponseWithId {
    type Error = anyhow::Error;

    fn try_from(proto: offchain::Response) -> std::result::Result<Self, Self::Error> {
        Ok(ResponseWithId {
            request_id: proto.request_id,
            response: proto.response.context("response missing")?.try_into()?,
        })
    }
}

impl From<ResponseWithId> for offchain::Response {
    fn from(val: ResponseWithId) -> Self {
        offchain::Response {
            request_id: val.request_id,
            response: Some(val.response.into()),
        }
    }
}

impl ResponseWithId {
    pub fn encode_to_vec(self: ResponseWithId) -> Vec<u8> {
        offchain::Response::from(self).encode_to_vec()
    }
}

pub fn stream_name(combo_config_digest: ConfigDigest) -> String {
    let combo_config_digest_without_0x_prefix = {
        let with_0x = combo_config_digest.to_string();
        with_0x
            .strip_prefix("0x")
            .expect("unexpected lack of 0x prefix")
            .to_string()
    };

    format!("ccip-rmn/v1_6/{}", combo_config_digest_without_0x_prefix)
}
