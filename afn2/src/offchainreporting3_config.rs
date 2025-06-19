#![allow(dead_code)]

use anyhow::Context;
use minieth::bytes::Bytes;
use prost::Message;

use crate::offchainreporting3_config_proto;

pub const OCR3_OFFCHAIN_CONFIG_VERSION: u64 = 30;

pub struct OffchainConfigProto {
    delta_progress_nanoseconds: u64,
    delta_resend_nanoseconds: u64,
    delta_initial_nanoseconds: u64,
    pub delta_round_nanoseconds: u64,
    delta_grace_nanoseconds: u64,
    delta_certified_commit_request_nanoseconds: u64,
    delta_stage_nanoseconds: u64,
    r_max: u64,
    s: Vec<u32>,
    offchain_public_keys: Vec<Bytes>,
    peer_ids: Vec<String>,
    pub reporting_plugin_config: Bytes,
    max_duration_query_nanoseconds: u64,
    max_duration_observation_nanoseconds: u64,
    max_duration_should_accept_attested_report_nanoseconds: u64,
    max_duration_should_transmit_accepted_report_nanoseconds: u64,
    shared_secret_encryptions: SharedSecretEncryptionsProto,
}

impl TryFrom<offchainreporting3_config_proto::OffchainConfigProto> for OffchainConfigProto {
    type Error = anyhow::Error;

    fn try_from(
        value: offchainreporting3_config_proto::OffchainConfigProto,
    ) -> Result<Self, Self::Error> {
        Ok(OffchainConfigProto {
            delta_progress_nanoseconds: value.delta_progress_nanoseconds,
            delta_resend_nanoseconds: value.delta_resend_nanoseconds,
            delta_initial_nanoseconds: value.delta_initial_nanoseconds,
            delta_round_nanoseconds: value.delta_round_nanoseconds,
            delta_grace_nanoseconds: value.delta_grace_nanoseconds,
            delta_certified_commit_request_nanoseconds: value
                .delta_certified_commit_request_nanoseconds,
            delta_stage_nanoseconds: value.delta_stage_nanoseconds,
            r_max: value.r_max,
            s: value.s,
            offchain_public_keys: value
                .offchain_public_keys
                .into_iter()
                .map(Bytes::from)
                .collect(),
            peer_ids: value.peer_ids,
            reporting_plugin_config: Bytes::from(value.reporting_plugin_config),
            max_duration_query_nanoseconds: value.max_duration_query_nanoseconds,
            max_duration_observation_nanoseconds: value.max_duration_observation_nanoseconds,
            max_duration_should_accept_attested_report_nanoseconds: value
                .max_duration_should_accept_attested_report_nanoseconds,
            max_duration_should_transmit_accepted_report_nanoseconds: value
                .max_duration_should_transmit_accepted_report_nanoseconds,
            shared_secret_encryptions: SharedSecretEncryptionsProto::from(
                value
                    .shared_secret_encryptions
                    .context("shared_secret_encryptions missing")?,
            ),
        })
    }
}

struct SharedSecretEncryptionsProto {
    diffie_hellman_point: Bytes,
    shared_secret_hash: Bytes,
    encryptions: Vec<Bytes>,
}

impl From<offchainreporting3_config_proto::SharedSecretEncryptionsProto>
    for SharedSecretEncryptionsProto
{
    fn from(value: offchainreporting3_config_proto::SharedSecretEncryptionsProto) -> Self {
        SharedSecretEncryptionsProto {
            diffie_hellman_point: Bytes::from(value.diffie_hellman_point),
            shared_secret_hash: Bytes::from(value.shared_secret_hash),
            encryptions: value.encryptions.into_iter().map(Bytes::from).collect(),
        }
    }
}

impl OffchainConfigProto {
    pub fn decode(slice: &[u8]) -> anyhow::Result<Self> {
        let proto = offchainreporting3_config_proto::OffchainConfigProto::decode(slice)?;
        proto.try_into()
    }
}
