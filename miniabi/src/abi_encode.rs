use minieth::{
    bytes::{Address, Bytes, Bytes32},
    u256::U256,
};

#[derive(Debug)]
pub enum StaticValue {
    Bool(bool),
    U256(U256),
    U64(u64),
    Address(Address),
    Bytes32(Bytes32),
    StaticTuple(Vec<StaticValue>),
}

#[derive(Debug)]
pub enum DynamicValue {
    Bytes(Bytes),
    DynamicArrayOfStaticValue(Vec<StaticValue>),
}

#[derive(Debug)]
pub enum ValueToEncode {
    Static(StaticValue),
    Dynamic(DynamicValue),
}

fn pad_right_to_32(v: &[u8]) -> Vec<u8> {
    let mut v = v.to_vec();
    let pad_len = (32 - v.len() % 32) % 32;
    v.resize(v.len() + pad_len, 0);
    v
}

fn combine_head_and_tail(head: usize, tail: &[u8], buf: &mut Vec<u8>) {
    U256::from(head as u128).abi_encode_into(buf);
    buf.extend(tail);
}

fn calc_offset(prefix_heads_size_in_bytes: usize, prefix_tails_size_in_bytes: usize) -> usize {
    prefix_heads_size_in_bytes + prefix_tails_size_in_bytes + 32
}

pub trait AbiEncode: Sized {
    fn abi_encode_into(&self, bytes: &mut Vec<u8>);

    fn abi_encode(&self) -> Bytes {
        let mut bytes = Vec::new();
        self.abi_encode_into(&mut bytes);
        bytes.into()
    }
}

impl AbiEncode for bool {
    fn abi_encode_into(&self, bytes: &mut Vec<u8>) {
        U256::from(*self).abi_encode_into(bytes)
    }
}
impl AbiEncode for U256 {
    fn abi_encode_into(&self, bytes: &mut Vec<u8>) {
        bytes.extend(self.to_be_bytes())
    }
}
impl AbiEncode for u64 {
    fn abi_encode_into(&self, bytes: &mut Vec<u8>) {
        U256::from(*self).abi_encode_into(bytes)
    }
}

impl AbiEncode for Address {
    fn abi_encode_into(&self, bytes: &mut Vec<u8>) {
        bytes.extend(&[0; 12]);
        bytes.extend(self.as_slice());
    }
}

impl AbiEncode for Bytes32 {
    fn abi_encode_into(&self, bytes: &mut Vec<u8>) {
        let data = <[u8; 32]>::from(*self);
        bytes.extend(&data)
    }
}

fn encode_bytes_into_head_and_tail(bytes: &Bytes, data_offset: usize) -> (usize, Vec<u8>) {
    let mut tail_encoded = vec![];
    let len = U256::from(u128::try_from(bytes.len()).unwrap());
    let right_padded_value = pad_right_to_32(bytes.as_ref());
    len.abi_encode_into(&mut tail_encoded);
    tail_encoded.extend(&right_padded_value);
    (data_offset, tail_encoded)
}

fn encode_bytes(v: &Bytes, buf: &mut Vec<u8>) {
    let (head, tail) = encode_bytes_into_head_and_tail(v, calc_offset(0, 0));
    combine_head_and_tail(head, &tail, buf)
}

fn encode_static_tuple(static_tuple: &[StaticValue], bytes: &mut Vec<u8>) {
    for v in static_tuple {
        v.abi_encode_into(bytes)
    }
}

fn encode_dynamic_array_of_static_value_into_head_and_tail(
    v: &[StaticValue],
    data_offset: usize,
) -> (usize, Vec<u8>) {
    let mut tail_encoded = vec![];
    U256::from(v.len() as u128).abi_encode_into(&mut tail_encoded);
    encode_static_tuple(v, &mut tail_encoded);
    (data_offset, tail_encoded)
}

fn encode_dynamic_array_of_static_value(v: &[StaticValue], buf: &mut Vec<u8>) {
    let (head, tail) =
        encode_dynamic_array_of_static_value_into_head_and_tail(v, calc_offset(0, 0));
    combine_head_and_tail(head, &tail, buf);
}

impl AbiEncode for StaticValue {
    fn abi_encode_into(&self, bytes: &mut Vec<u8>) {
        match self {
            StaticValue::Bool(v) => v.abi_encode_into(bytes),
            StaticValue::U256(v) => v.abi_encode_into(bytes),
            StaticValue::U64(v) => v.abi_encode_into(bytes),
            StaticValue::Address(v) => v.abi_encode_into(bytes),
            StaticValue::Bytes32(v) => v.abi_encode_into(bytes),
            StaticValue::StaticTuple(v) => encode_static_tuple(v, bytes),
        }
    }
}

impl AbiEncode for DynamicValue {
    fn abi_encode_into(&self, bytes: &mut Vec<u8>) {
        match self {
            DynamicValue::Bytes(v) => encode_bytes(v, bytes),
            DynamicValue::DynamicArrayOfStaticValue(v) => {
                encode_dynamic_array_of_static_value(v, bytes)
            }
        }
    }
}

impl AbiEncode for ValueToEncode {
    fn abi_encode_into(&self, bytes: &mut Vec<u8>) {
        match self {
            ValueToEncode::Static(static_value) => static_value.abi_encode_into(bytes),
            ValueToEncode::Dynamic(dynamic_value) => dynamic_value.abi_encode_into(bytes),
        }
    }
}
