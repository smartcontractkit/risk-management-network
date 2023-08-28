use crate::bytes::{prepend, Address, Bytes, Bytes32};
use crate::ecdsa::{EthereumSignature, EthereumSigner};
use crate::keccak::keccak256;
use crate::rlp::{RlpEncodable, RlpStream};
use crate::u256::U256;

const EIP_1559_TX_TYPE: u8 = 2;

#[derive(Clone)]
pub struct AccessListRecord {
    address: Address,
    storage_keys: Vec<Bytes32>,
}

impl RlpEncodable for AccessListRecord {
    fn to_rlp_bytes(&self) -> Vec<u8> {
        let mut rlp = RlpStream::new();
        rlp.push(&self.address);
        rlp.push(&self.storage_keys);
        rlp.finalize_with_length_prefix()
    }
}

#[derive(Clone)]
pub struct Eip1559TransactionRequest {
    pub chain_id: u64,
    pub nonce: U256,
    pub max_priority_fee_per_gas: U256,
    pub max_fee_per_gas: U256,
    pub gas: U256,
    pub to: Address,
    pub value: U256,
    pub data: Bytes,
    pub access_list: Vec<AccessListRecord>,
}

pub struct Eip1559Transaction {
    pub req: Eip1559TransactionRequest,
    pub sig: EthereumSignature,
}

impl RlpEncodable for Eip1559TransactionRequest {
    fn to_rlp_bytes(&self) -> Vec<u8> {
        let mut rlp = RlpStream::new();
        rlp.push(&self.chain_id);
        rlp.push(&self.nonce);
        rlp.push(&self.max_priority_fee_per_gas);
        rlp.push(&self.max_fee_per_gas);
        rlp.push(&self.gas);
        rlp.push(&self.to);
        rlp.push(&self.value);
        rlp.push(&self.data);
        rlp.push(&self.access_list);
        rlp.finalize_without_length_prefix()
    }
}

impl Eip1559TransactionRequest {
    fn to_signing_hash(&self) -> [u8; 32] {
        let mut rlp = RlpStream::new();
        rlp.extend(&self.to_rlp_bytes());
        keccak256(&prepend(
            EIP_1559_TX_TYPE,
            &rlp.finalize_with_length_prefix(),
        ))
    }

    pub fn sign(&self, signer: &dyn EthereumSigner) -> Eip1559Transaction {
        let sig = signer.sign_hashed(self.to_signing_hash());
        Eip1559Transaction {
            req: self.clone(),
            sig,
        }
    }
}

impl Eip1559Transaction {
    pub fn to_rlp_bytes(&self) -> Vec<u8> {
        let mut rlp = RlpStream::new();
        rlp.push(&self.req);
        rlp.push(&self.sig);
        prepend(EIP_1559_TX_TYPE, &rlp.finalize_with_length_prefix())
    }
}

#[derive(Clone)]
pub struct LegacyTransactionRequest {
    pub chain_id: u64,
    pub nonce: U256,
    pub gas_price: U256,
    pub gas: U256,
    pub to: Address,
    pub value: U256,
    pub data: Bytes,
}

impl RlpEncodable for LegacyTransactionRequest {
    fn to_rlp_bytes(&self) -> Vec<u8> {
        let mut rlp = RlpStream::new();
        rlp.push(&self.nonce);
        rlp.push(&self.gas_price);
        rlp.push(&self.gas);
        rlp.push(&self.to);
        rlp.push(&self.value);
        rlp.push(&self.data);
        rlp.finalize_without_length_prefix()
    }
}

impl LegacyTransactionRequest {
    fn to_signing_hash(&self) -> [u8; 32] {
        let mut rlp = RlpStream::new();
        rlp.extend(&self.to_rlp_bytes());
        rlp.push(&self.chain_id);
        rlp.push(&U256::from(0u8));
        rlp.push(&U256::from(0u8));
        keccak256(&rlp.finalize_with_length_prefix())
    }

    pub fn sign(&self, signer: &dyn EthereumSigner) -> LegacyTransaction {
        let sig = signer.sign_hashed(self.to_signing_hash());
        LegacyTransaction {
            req: self.clone(),
            sig,
        }
    }
}

pub struct LegacyTransaction {
    pub req: LegacyTransactionRequest,
    pub sig: EthereumSignature,
}

impl LegacyTransaction {
    pub fn to_rlp_bytes(&self) -> Vec<u8> {
        let mut rlp = RlpStream::new();
        rlp.push(&self.req);
        rlp.push(&self.sig.to_legacy(self.req.chain_id));
        rlp.finalize_with_length_prefix()
    }
}
