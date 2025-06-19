#![allow(unused_imports)]
pub use afn::chains::ChainSelector;
pub use afn::common::ContractCall;
pub use afn::common::LogSignature;
pub use afn::common::UncheckedDecodeLog;
pub use anyhow::Result;
pub use miniabi::abi_decode::AbiDecode;
pub use miniabi::{
    abi_decode::{AbiDecodeError, AbiTypeToDecode},
    abi_encode::AbiEncode,
    types::{Type, Value},
};
pub use minieth::rpc::EVMLog;
pub use minieth::{
    bytes::{Address, Bytes, Bytes32},
    keccak::keccak256,
    u256::U256,
};

pub trait ContractCallReturnValue {
    type ReturnValue: AbiDecode;
}
