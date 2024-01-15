use crate::abi_decode::AbiDecodeError;
use crate::types::{Type, Value};
use minieth::{
    bytes::{Address, Bytes, Bytes32},
    u256::U256,
};

struct DecodeResult {
    pub value: Value,
    pub new_cursor: usize,
    pub size_in_bytes: usize,
}

impl Type {
    pub(crate) fn decode(&self, bytes: &[u8]) -> Result<Value, AbiDecodeError> {
        let v = self.decode_helper(bytes, 0, true)?;
        if v.size_in_bytes != bytes.len() {
            return Err(AbiDecodeError::InvalidEncodingSize);
        }
        Ok(v.value)
    }

    #[cfg(feature = "arbitrary")]
    pub fn decode_fuzzhelper(&self, bytes: &[u8]) -> Result<Value, AbiDecodeError> {
        self.decode(bytes)
    }

    fn decode_helper(
        &self,
        bytes: &[u8],
        cursor: usize,
        is_top_level: bool,
    ) -> Result<DecodeResult, AbiDecodeError> {
        match self {
            Type::Bool => decode_bool_helper(bytes, cursor),
            Type::U256 => decode_u256_helper(bytes, cursor),
            Type::U128 => decode_u128_helper(bytes, cursor),
            Type::U64 => decode_u64_helper(bytes, cursor),
            Type::U32 => decode_u32_helper(bytes, cursor),
            Type::U16 => decode_u16_helper(bytes, cursor),
            Type::U8 => decode_u8_helper(bytes, cursor),
            Type::Address => decode_address_helper(bytes, cursor),
            Type::Bytes32 => decode_bytes32_helper(bytes, cursor),
            Type::Bytes => decode_bytes_helper(bytes, cursor),
            Type::String => decode_string_helper(bytes, cursor),
            Type::DynamicArray(type_to_decode) => {
                decode_dynamic_array(type_to_decode, bytes, cursor)
            }
            Type::Tuple(tuple) => {
                let is_dynamic = !(is_top_level || self.is_static());
                decode_tuple_helper(tuple, bytes, cursor, is_dynamic)
            }
        }
    }
}

fn read_first_32_bytes(bytes: &[u8], cursor: usize) -> Result<[u8; 32], AbiDecodeError> {
    if usize::saturating_sub(bytes.len(), cursor) < 32 {
        return Err(AbiDecodeError::InvalidEncodingSize);
    }
    bytes[cursor..cursor + 32]
        .try_into()
        .or(Err(AbiDecodeError::InvalidEncodingSize))
}

fn are_all_zeros(v: &[u8]) -> bool {
    v.iter().all(|v| *v == 0u8)
}

fn bool_from_32bytes(v: [u8; 32]) -> Result<bool, AbiDecodeError> {
    if !are_all_zeros(&v[..31]) {
        return Err(AbiDecodeError::InvalidBoolEncoding);
    }

    match v[31] {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(AbiDecodeError::InvalidBoolEncoding),
    }
}

fn decode_bool_helper(bytes: &[u8], cursor: usize) -> Result<DecodeResult, AbiDecodeError> {
    let v: bool = bool_from_32bytes(read_first_32_bytes(bytes, cursor)?)?;
    Ok(DecodeResult {
        value: Value::Bool(v),
        new_cursor: cursor + 32,
        size_in_bytes: 32,
    })
}
fn decode_u256_helper(bytes: &[u8], cursor: usize) -> Result<DecodeResult, AbiDecodeError> {
    let v: U256 = U256::from_be_bytes(read_first_32_bytes(bytes, cursor)?);
    Ok(DecodeResult {
        value: Value::U256(v),
        new_cursor: cursor + 32,
        size_in_bytes: 32,
    })
}

macro_rules! impl_decode_uint_helper {
    ($constructor: ident, $type: ident, $func_name:ident) => {
        fn $func_name(bytes: &[u8], cursor: usize) -> Result<DecodeResult, AbiDecodeError> {
            let first_32_bytes = read_first_32_bytes(bytes, cursor)?;
            let (padding, data) = first_32_bytes.split_at(32 - std::mem::size_of::<$type>());
            if !are_all_zeros(&padding) {
                return Err(AbiDecodeError::InvalidUintEncoding);
            }
            let v = $type::from_be_bytes(data.try_into().unwrap())
                .try_into()
                .or(Err(AbiDecodeError::InvalidUintEncoding))?;
            Ok(DecodeResult {
                value: Value::$constructor(v),
                new_cursor: cursor + 32,
                size_in_bytes: 32,
            })
        }
    };
}
impl_decode_uint_helper!(U128, u128, decode_u128_helper);
impl_decode_uint_helper!(U64, u64, decode_u64_helper);
impl_decode_uint_helper!(U32, u32, decode_u32_helper);
impl_decode_uint_helper!(U16, u16, decode_u16_helper);
impl_decode_uint_helper!(U8, u8, decode_u8_helper);

fn address_from_32_bytes(v: [u8; 32]) -> Result<Address, AbiDecodeError> {
    if !are_all_zeros(&v[..12]) {
        return Err(AbiDecodeError::InvalidAddressEncoding);
    }
    let bytes: [u8; 20] = v[12..32]
        .try_into()
        .or(Err(AbiDecodeError::InvalidAddressEncoding))?;
    Ok(Address::from(bytes))
}

fn decode_address_helper(bytes: &[u8], cursor: usize) -> Result<DecodeResult, AbiDecodeError> {
    let v: Address = address_from_32_bytes(read_first_32_bytes(bytes, cursor)?)?;
    Ok(DecodeResult {
        value: Value::Address(v),
        new_cursor: cursor + 32,
        size_in_bytes: 32,
    })
}

fn decode_bytes32_helper(bytes: &[u8], cursor: usize) -> Result<DecodeResult, AbiDecodeError> {
    let first_32_bytes: [u8; 32] = read_first_32_bytes(bytes, cursor)?;
    let v: Bytes32 = first_32_bytes.into();
    Ok(DecodeResult {
        value: Value::Bytes32(v),
        new_cursor: cursor + 32,
        size_in_bytes: 32,
    })
}

fn usize_from_32_bytes(v: [u8; 32]) -> Result<usize, AbiDecodeError> {
    if !are_all_zeros(&v[..16]) {
        return Err(AbiDecodeError::InvalidUsizeEncoding);
    }
    let bytes: [u8; 16] = v[16..32]
        .try_into()
        .or(Err(AbiDecodeError::InvalidUsizeEncoding))?;
    let val: usize = u128::from_be_bytes(bytes)
        .try_into()
        .or(Err(AbiDecodeError::InvalidUsizeEncoding))?;
    Ok(val)
}

fn read_prefix_bytes(bytes: &[u8], cursor: usize, length: usize) -> Result<&[u8], AbiDecodeError> {
    if usize::saturating_sub(bytes.len(), cursor) < length {
        return Err(AbiDecodeError::InvalidEncodingSize);
    }
    Ok(&bytes[cursor..cursor + length])
}

fn round_up_to_multiple_of_32(v: usize) -> Result<usize, AbiDecodeError> {
    if usize::MAX - 31 < v {
        return Err(AbiDecodeError::InvalidUsizeEncoding);
    }
    Ok((v + 31) / 32 * 32)
}

fn decode_bytes_helper(bytes: &[u8], cursor: usize) -> Result<DecodeResult, AbiDecodeError> {
    let data_offset: usize = usize_from_32_bytes(read_first_32_bytes(bytes, cursor)?)?;
    if data_offset > bytes.len() || data_offset < 32 {
        return Err(AbiDecodeError::InvalidUsizeEncoding);
    }
    let length = usize_from_32_bytes(read_first_32_bytes(bytes, data_offset)?)?;
    let rounded_length = round_up_to_multiple_of_32(length)?;
    let data = read_prefix_bytes(bytes, data_offset + 32, rounded_length)?;
    if !are_all_zeros(&data[length..rounded_length]) {
        return Err(AbiDecodeError::InvalidBytesEncoding);
    }
    let bytes = Bytes::from(data[..length].to_vec());
    Ok(DecodeResult {
        value: Value::Bytes(bytes),
        new_cursor: cursor + 32,
        size_in_bytes: 64 + rounded_length,
    })
}

fn decode_string_helper(bytes: &[u8], cursor: usize) -> Result<DecodeResult, AbiDecodeError> {
    match decode_bytes_helper(bytes, cursor) {
        Err(AbiDecodeError::InvalidBytesEncoding) => Err(AbiDecodeError::InvalidStringEncoding),
        Err(err) => Err(err),
        Ok(DecodeResult {
            value: Value::Bytes(bytes),
            new_cursor,
            size_in_bytes,
        }) => Ok(DecodeResult {
            value: Value::String(
                String::from_utf8(bytes.to_vec()).map_err(|_| AbiDecodeError::InvalidValueError)?,
            ),
            new_cursor,
            size_in_bytes,
        }),
        Ok(_) => Err(AbiDecodeError::InvalidValueError),
    }
}

fn decode_dynamic_array(
    type_to_decode: &Type,
    bytes: &[u8],
    cursor: usize,
) -> Result<DecodeResult, AbiDecodeError> {
    let data_offset: usize = usize_from_32_bytes(read_first_32_bytes(bytes, cursor)?)?;
    if data_offset > bytes.len() || data_offset < 32 {
        return Err(AbiDecodeError::InvalidUsizeEncoding);
    }
    let len_offset = data_offset;
    let length = usize_from_32_bytes(read_first_32_bytes(bytes, len_offset)?)?;
    let tail_offset = len_offset + 32;
    if tail_offset > bytes.len() {
        return Err(AbiDecodeError::InvalidUsizeEncoding);
    }
    let bytes = &bytes[tail_offset..];

    let mut data_cursor = 0;
    let mut total_size_in_bytes = 0;
    let mut res: Vec<Value> = vec![];
    for _i in 0..length {
        let v = type_to_decode.decode_helper(bytes, data_cursor, false)?;
        res.push(v.value);
        data_cursor = v.new_cursor;
        total_size_in_bytes += v.size_in_bytes;
    }
    Ok(DecodeResult {
        value: Value::DynamicArray(res),
        new_cursor: cursor + 32,
        size_in_bytes: 64 + total_size_in_bytes,
    })
}

fn decode_tuple_helper(
    tuple: &[Type],
    bytes: &[u8],
    cursor: usize,
    is_dynamic: bool,
) -> Result<DecodeResult, AbiDecodeError> {
    if tuple.is_empty() {
        return Err(AbiDecodeError::ZeroSizedTypeError);
    }
    let mut total_size_in_bytes = 0;

    let (bytes, mut data_cursor) = if is_dynamic {
        let data_offset: usize = usize_from_32_bytes(read_first_32_bytes(bytes, cursor)?)?;
        if data_offset > bytes.len() || data_offset < 32 {
            return Err(AbiDecodeError::InvalidUsizeEncoding);
        }

        total_size_in_bytes += 32;

        let bytes = &bytes[data_offset..];
        (bytes, 0)
    } else {
        (bytes, cursor)
    };

    let mut res: Vec<Value> = vec![];
    for type_to_decode in tuple.iter() {
        let v = type_to_decode.decode_helper(bytes, data_cursor, false)?;
        res.push(v.value);
        data_cursor = v.new_cursor;
        total_size_in_bytes += v.size_in_bytes;
    }

    Ok(DecodeResult {
        value: Value::Tuple(res),
        new_cursor: if is_dynamic { cursor + 32 } else { data_cursor },
        size_in_bytes: total_size_in_bytes,
    })
}
