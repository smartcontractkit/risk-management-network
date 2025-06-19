#![forbid(unsafe_code)]
#![warn(clippy::all)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::let_and_return)]
#![allow(clippy::if_same_then_else)]
#![allow(clippy::nonminimal_bool)]
#![allow(rustdoc::private_intra_doc_links)]
#![allow(clippy::empty_line_after_doc_comments)]

mod rageproxy_proto {
    include!(concat!(env!("OUT_DIR"), "/rageproxy.rs"));
}

mod offchain_proto {
    include!(concat!(env!("OUT_DIR"), "/rmn_offchain.rs"));
}

#[cfg(feature = "arbitrary")]
pub mod offchain_proto_ {
    pub use crate::offchain_proto::*;
}

mod keystore_proto {
    include!(concat!(env!("OUT_DIR"), "/keystore.rs"));
}

mod offchainreporting3_config_proto {
    include!(concat!(env!("OUT_DIR"), "/offchainreporting3_config.rs"));
}

pub mod cesm;
pub mod config_digest;
pub mod contracts;
pub mod keystore2;
pub mod limits;
pub mod offchain;
mod offchainreporting3_config;
pub mod ragep2p;
pub mod rageproxy;
pub mod signing;
