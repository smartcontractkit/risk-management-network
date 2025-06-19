use crate::config_digest::ConfigDigest;
use crate::ragep2p::{Bootstrapper, PeerId};
use crate::rageproxy_proto as rageproxy;

use anyhow::anyhow;
use prost::Message;

const MB: u32 = 1_000_000;
pub const MAX_RAGEPROXY_INCOMING_MESSAGE_SIZE: u32 = 10 * MB;

pub struct MessageProxyToProxyTarget {
    pub sender_peer_id: PeerId,
    pub stream_name: String,
    pub payload: Vec<u8>,
}

impl TryFrom<rageproxy::MessageProxyToProxyTarget> for MessageProxyToProxyTarget {
    type Error = anyhow::Error;

    fn try_from(
        proto: rageproxy::MessageProxyToProxyTarget,
    ) -> std::result::Result<Self, Self::Error> {
        Ok(MessageProxyToProxyTarget {
            sender_peer_id: proto.sender_peer_id.try_into()?,
            stream_name: proto.stream_name,
            payload: proto.payload,
        })
    }
}

pub enum ProxyToProxyTarget {
    MessageProxyToProxyTarget(MessageProxyToProxyTarget),
}

impl TryFrom<rageproxy::ProxyToProxyTarget> for ProxyToProxyTarget {
    type Error = anyhow::Error;

    fn try_from(proto: rageproxy::ProxyToProxyTarget) -> std::result::Result<Self, Self::Error> {
        match proto.msg {
            Some(rageproxy::proxy_to_proxy_target::Msg::MessageProxyToProxyTarget(proto)) => Ok(
                ProxyToProxyTarget::MessageProxyToProxyTarget(proto.try_into()?),
            ),
            None => Err(anyhow!(
                "received empty rageproxy message, this should never happen"
            )),
        }
    }
}

impl ProxyToProxyTarget {
    pub fn decode(slice: &[u8]) -> anyhow::Result<Self> {
        let proto = rageproxy::ProxyToProxyTarget::decode(slice)?;
        proto.try_into()
    }
}

pub struct MessageProxyTargetToProxy {
    pub receiver_peer_id: PeerId,
    pub stream_name: String,
    pub payload: Vec<u8>,
}

impl From<MessageProxyTargetToProxy> for rageproxy::MessageProxyTargetToProxy {
    fn from(val: MessageProxyTargetToProxy) -> Self {
        rageproxy::MessageProxyTargetToProxy {
            receiver_peer_id: val.receiver_peer_id.to_vec(),
            stream_name: val.stream_name,
            payload: val.payload,
        }
    }
}

impl From<Bootstrapper> for rageproxy::Bootstrapper {
    fn from(val: Bootstrapper) -> Self {
        rageproxy::Bootstrapper {
            peer_id: val.peer_id.to_vec(),
            addrs: val.addrs,
        }
    }
}

#[derive(Debug)]
pub struct AddGroup {
    pub config_digest: ConfigDigest,
    pub peer_ids: Vec<PeerId>,
    pub bootstrappers: Vec<Bootstrapper>,
}

impl From<AddGroup> for rageproxy::AddGroup {
    fn from(val: AddGroup) -> Self {
        rageproxy::AddGroup {
            config_digest: val.config_digest.to_vec(),
            peer_ids: val.peer_ids.into_iter().map(|id| id.to_vec()).collect(),
            bootstrappers: val.bootstrappers.into_iter().map(|b| b.into()).collect(),
        }
    }
}

pub struct RemoveGroup {
    pub config_digest: ConfigDigest,
}

impl From<RemoveGroup> for rageproxy::RemoveGroup {
    fn from(val: RemoveGroup) -> Self {
        rageproxy::RemoveGroup {
            config_digest: val.config_digest.to_vec(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TokenBucketParams {
    pub rate: f64,
    pub capacity: u32,
}

impl From<TokenBucketParams> for rageproxy::TokenBucketParams {
    fn from(val: TokenBucketParams) -> Self {
        rageproxy::TokenBucketParams {
            rate: val.rate,
            capacity: val.capacity,
        }
    }
}

#[derive(Debug)]
pub struct NewStream {
    pub other: PeerId,
    pub stream_name: String,
    pub outgoing_buffer_size: u64,
    pub incoming_buffer_size: u64,
    pub max_message_length: u64,
    pub messages_limit: TokenBucketParams,
    pub bytes_limit: TokenBucketParams,
}

impl From<NewStream> for rageproxy::NewStream {
    fn from(val: NewStream) -> Self {
        rageproxy::NewStream {
            other: val.other.to_vec(),
            stream_name: val.stream_name,
            outgoing_buffer_size: val.outgoing_buffer_size,
            incoming_buffer_size: val.incoming_buffer_size,
            max_message_length: val.max_message_length,
            messages_limit: Some(val.messages_limit.into()),
            bytes_limit: Some(val.bytes_limit.into()),
        }
    }
}

pub struct CloseStream {
    pub other: PeerId,
    pub stream_name: String,
}

impl From<CloseStream> for rageproxy::CloseStream {
    fn from(val: CloseStream) -> Self {
        rageproxy::CloseStream {
            other: val.other.to_vec(),
            stream_name: val.stream_name,
        }
    }
}

#[allow(clippy::enum_variant_names)]
pub enum ProxyTargetToProxy {
    AddGroup(AddGroup),
    RemoveGroup(RemoveGroup),
    NewStream(NewStream),
    CloseStream(CloseStream),
    MessageProxyTargetToProxy(MessageProxyTargetToProxy),
}

impl From<ProxyTargetToProxy> for rageproxy::ProxyTargetToProxy {
    fn from(val: ProxyTargetToProxy) -> Self {
        match val {
            ProxyTargetToProxy::AddGroup(val) => rageproxy::ProxyTargetToProxy {
                msg: Some(rageproxy::proxy_target_to_proxy::Msg::AddGroup(val.into())),
            },
            ProxyTargetToProxy::RemoveGroup(val) => rageproxy::ProxyTargetToProxy {
                msg: Some(rageproxy::proxy_target_to_proxy::Msg::RemoveGroup(
                    val.into(),
                )),
            },
            ProxyTargetToProxy::NewStream(val) => rageproxy::ProxyTargetToProxy {
                msg: Some(rageproxy::proxy_target_to_proxy::Msg::NewStream(val.into())),
            },
            ProxyTargetToProxy::CloseStream(val) => rageproxy::ProxyTargetToProxy {
                msg: Some(rageproxy::proxy_target_to_proxy::Msg::CloseStream(
                    val.into(),
                )),
            },
            ProxyTargetToProxy::MessageProxyTargetToProxy(val) => rageproxy::ProxyTargetToProxy {
                msg: Some(
                    rageproxy::proxy_target_to_proxy::Msg::MessageProxyTargetToProxy(val.into()),
                ),
            },
        }
    }
}

impl ProxyTargetToProxy {
    pub fn encode_to_vec(self: ProxyTargetToProxy) -> Vec<u8> {
        rageproxy::ProxyTargetToProxy::from(self).encode_to_vec()
    }
}
