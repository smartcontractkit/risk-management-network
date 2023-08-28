#![forbid(unsafe_code)]
#![warn(clippy::all)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::let_and_return)]
#![allow(clippy::if_same_then_else)]
#![allow(rustdoc::private_intra_doc_links)]

mod afn_contract;
pub mod afn_voting_manager;
mod cached_rpc;
mod chain_selector;
mod chain_state;
mod chain_status;
mod chain_status_worker;
mod commit_report;
mod commit_store;
pub mod common;
pub mod config;
mod config_sanity_check;
mod contract_event_state_machine;
pub mod curse_beacon;
pub mod encryption;
mod evm2evm_offramp;
mod evm2evm_onramp;
mod hashable;
mod inflight_root_cache;
pub mod key_types;
mod lane_bless_status;
mod lane_state;
mod merkle;
mod offramp_anomaly_detector;
mod onchain_config_discovery_worker;
mod onramp_traits;
mod permutation;
mod reaper;
pub mod state;
mod vote_to_bless_worker;
mod vote_to_curse_worker;
pub mod worker;
