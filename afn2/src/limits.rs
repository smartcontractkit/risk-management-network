use std::{cmp::max, time::Duration};

use anyhow::Context;

use crate::{
    config_digest::ConfigDigest,
    offchain::{
        AttributedSignedObservation, FixedDestLaneUpdate, FixedDestLaneUpdateRequest,
        ObservationRequest, ReportSignatureRequest,
    },
    rageproxy::{self, TokenBucketParams},
};

const RAGEP2P_MAX_MESSAGE_LENGTH: u64 = 1024 * 1024 * 1024;

#[derive(Debug)]
pub struct NewStreamLimits {
    pub outgoing_buffer_size: u64,
    pub incoming_buffer_size: u64,
    pub max_message_length: u64,
    pub messages_limit: TokenBucketParams,
    pub bytes_limit: TokenBucketParams,
}

pub fn new_stream_limits(
    estimated_round_interval: Duration,
    num_rmn_source_chains: usize,
    num_rmn_nodes: usize,
) -> anyhow::Result<NewStreamLimits> {
    fn safe_i32(x: usize) -> anyhow::Result<usize> {
        let _ = i32::try_from(x).context("i32 overflow")?;
        Ok(x)
    }

    fn add<const N: usize>(xs: [usize; N]) -> anyhow::Result<usize> {
        safe_i32(
            xs.iter()
                .try_fold(0usize, |acc, &x| acc.checked_add(x))
                .ok_or(anyhow::anyhow!("usize overflow"))?,
        )
    }

    fn mul<const N: usize>(xs: [usize; N]) -> anyhow::Result<usize> {
        safe_i32(
            xs.iter()
                .try_fold(1usize, |acc, &x| acc.checked_mul(x))
                .ok_or(anyhow::anyhow!("usize overflow"))?,
        )
    }

    let messages_limit = {
        let num_msgs_per_round = 2;

        TokenBucketParams {
            rate: ((num_msgs_per_round as f64) / estimated_round_interval.as_secs_f64())
                * RATE_SCALE,
            capacity: mul([num_msgs_per_round, CAPACITY_SCALE])?
                .try_into()
                .context("overflow")?,
        }
    };

    const PROTO_OVERHEAD: usize = 256;
    const RATE_SCALE: f64 = 1.2;
    const CAPACITY_SCALE: usize = 3;

    const LEN_REQUEST_ID: usize = size_of::<u64>();

    let max_len_observation_request = add([
        PROTO_OVERHEAD,
        LEN_REQUEST_ID,
        size_of::<ObservationRequest>(),
        mul([
            num_rmn_source_chains,
            size_of::<FixedDestLaneUpdateRequest>(),
        ])?,
    ])?;

    let max_len_report_signature_request = {
        let max_len_attributed_signed_observation = add([
            PROTO_OVERHEAD,
            size_of::<AttributedSignedObservation>(),
            mul([num_rmn_source_chains, size_of::<FixedDestLaneUpdate>()])?,
        ])?;

        add([
            PROTO_OVERHEAD,
            LEN_REQUEST_ID,
            size_of::<ReportSignatureRequest>(),
            mul([num_rmn_nodes, max_len_attributed_signed_observation])?,
        ])?
    };

    let bytes_limit = {
        let max_sum_len_inbound_per_round = add([
            max_len_observation_request,
            max_len_report_signature_request,
        ])?;

        TokenBucketParams {
            rate: (max_sum_len_inbound_per_round as f64 / estimated_round_interval.as_secs_f64()),
            capacity: mul([max_sum_len_inbound_per_round, CAPACITY_SCALE])?
                .try_into()
                .context("overflow")?,
        }
    };

    let max_message_length: u64 = safe_i32(max(
        max_len_observation_request,
        max_len_report_signature_request,
    ))?
    .try_into()
    .context("overflow")?;

    if max_message_length > RAGEP2P_MAX_MESSAGE_LENGTH {
        return Err(anyhow::anyhow!(
            "max_message_length exceeds RAGEP2P_MAX_MESSAGE_LENGTH"
        ));
    }

    let max_inbound_rageproxy_message_length: u32 = add([
        PROTO_OVERHEAD,
        size_of::<rageproxy::ProxyToProxyTarget>(),
        crate::offchain::stream_name(ConfigDigest::default()).len(),
        max_message_length.try_into().context("overflow")?,
    ])?
    .try_into()
    .context("overflow")?;

    if max_inbound_rageproxy_message_length > rageproxy::MAX_RAGEPROXY_INCOMING_MESSAGE_SIZE {
        return Err(anyhow::anyhow!(
            "max_inbound_rageproxy_message exceeds MAX_RAGEPROXY_INCOMING_MESSAGE_SIZE"
        ));
    }

    Ok(NewStreamLimits {
        outgoing_buffer_size: 1,
        incoming_buffer_size: 1,
        max_message_length,
        messages_limit,
        bytes_limit,
    })
}
