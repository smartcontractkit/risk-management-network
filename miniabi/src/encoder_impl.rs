use crate::types::Value;
use minieth::{
    bytes::{Address, Bytes, Bytes32},
    u256::U256,
};

impl Value {
    pub(crate) fn encode_into(self, bytes: &mut Vec<u8>) {
        match self {
            Value::Tuple(_) => {
                self.encode_actual_data_into(bytes);
            }
            v => Value::Tuple(vec![v]).encode_actual_data_into(bytes),
        }
    }
}

fn pad_right_to_multiple_of_32_bytes(v: &[u8]) -> Vec<u8> {
    let mut v = v.to_vec();
    let pad_len = (32 - v.len() % 32) % 32;
    v.resize(v.len() + pad_len, 0);
    v
}

trait ActualDataEncode: Sized {
    fn encode_actual_data_into(self, bytes: &mut Vec<u8>);
    fn encode_actual_data(self) -> Bytes {
        let mut bytes = Vec::new();
        self.encode_actual_data_into(&mut bytes);
        bytes.into()
    }
}

impl ActualDataEncode for U256 {
    fn encode_actual_data_into(self, bytes: &mut Vec<u8>) {
        bytes.extend(self.to_be_bytes())
    }
}
macro_rules! impl_actual_data_encode_for_uint {
    ($type: ident) => {
        impl ActualDataEncode for $type {
            fn encode_actual_data_into(self, bytes: &mut Vec<u8>) {
                U256::from(self).encode_actual_data_into(bytes)
            }
        }
    };
}
impl_actual_data_encode_for_uint!(u128);
impl_actual_data_encode_for_uint!(u64);
impl_actual_data_encode_for_uint!(u32);
impl_actual_data_encode_for_uint!(u16);
impl_actual_data_encode_for_uint!(u8);

impl_actual_data_encode_for_uint!(bool);

impl ActualDataEncode for Address {
    fn encode_actual_data_into(self, bytes: &mut Vec<u8>) {
        bytes.extend(&[0; 12]);
        bytes.extend(self.as_slice());
    }
}
impl ActualDataEncode for Bytes32 {
    fn encode_actual_data_into(self, bytes: &mut Vec<u8>) {
        let data = <[u8; 32]>::from(self);
        bytes.extend(&data)
    }
}

impl ActualDataEncode for Bytes {
    fn encode_actual_data_into(self, bytes: &mut Vec<u8>) {
        let len = U256::try_from(self.len()).unwrap();
        let right_padded_value = pad_right_to_multiple_of_32_bytes(self.as_ref());
        len.encode_actual_data_into(bytes);
        bytes.extend(&right_padded_value);
    }
}

struct Tuple(Vec<Value>);

impl ActualDataEncode for Tuple {
    fn encode_actual_data_into(self, bytes: &mut Vec<u8>) {
        let tuple = self.0;
        let mut total_head_size_in_bytes = 0;
        let mut tuple_actual_data_encodings = Vec::new();
        for v in tuple {
            let mut enc = Vec::new();
            let is_static = v.is_static();
            v.encode_actual_data_into(&mut enc);
            if is_static {
                total_head_size_in_bytes += enc.len();
            } else {
                total_head_size_in_bytes += 32;
            }
            tuple_actual_data_encodings.push((is_static, enc));
        }
        let mut heads = Vec::new();
        let mut tails = Vec::new();
        for (is_static, enc) in tuple_actual_data_encodings.iter_mut() {
            if *is_static {
                heads.append(enc);
            } else {
                U256::try_from(total_head_size_in_bytes + tails.len())
                    .unwrap()
                    .encode_actual_data_into(&mut heads);
                tails.append(enc);
            }
        }
        bytes.append(&mut heads);
        bytes.append(&mut tails);
    }
}

struct DynamicArray(Vec<Value>);

impl ActualDataEncode for DynamicArray {
    fn encode_actual_data_into(self, bytes: &mut Vec<u8>) {
        U256::try_from(self.0.len())
            .unwrap()
            .encode_actual_data_into(bytes);
        Tuple(self.0).encode_actual_data_into(bytes);
    }
}

impl ActualDataEncode for String {
    fn encode_actual_data_into(self, bytes: &mut Vec<u8>) {
        Bytes::from(self.into_bytes()).encode_actual_data_into(bytes)
    }
}

impl ActualDataEncode for Value {
    fn encode_actual_data_into(self, bytes: &mut Vec<u8>) {
        match self {
            Value::Bool(v) => v.encode_actual_data_into(bytes),
            Value::U256(v) => v.encode_actual_data_into(bytes),
            Value::U128(v) => v.encode_actual_data_into(bytes),
            Value::U64(v) => v.encode_actual_data_into(bytes),
            Value::U32(v) => v.encode_actual_data_into(bytes),
            Value::U16(v) => v.encode_actual_data_into(bytes),
            Value::U8(v) => v.encode_actual_data_into(bytes),
            Value::Address(v) => v.encode_actual_data_into(bytes),
            Value::Bytes32(v) => v.encode_actual_data_into(bytes),
            Value::Bytes(v) => v.encode_actual_data_into(bytes),
            Value::String(v) => v.encode_actual_data_into(bytes),
            Value::Tuple(v) => Tuple(v).encode_actual_data_into(bytes),
            Value::DynamicArray(v) => DynamicArray(v).encode_actual_data_into(bytes),
        }
    }
}
