fn trim_bytes(us: &[u8]) -> &[u8] {
    let idx = us.iter().position(|b| b != &0).unwrap_or(us.len());
    &us[idx..]
}

pub fn rlp_encode_bytes(bytes: &[u8]) -> Vec<u8> {
    if bytes.len() == 1 && bytes[0] < 0x80 {
        return bytes.into();
    }
    let mut v = if bytes.len() <= 55 {
        vec![0x80 + (bytes.len() as u8)]
    } else {
        let len_be = bytes.len().to_be_bytes();
        let len_bytes = trim_bytes(&len_be);
        let mut v = vec![0xb7 + (len_bytes.len() as u8)];
        v.extend(len_bytes);
        v
    };
    v.extend(bytes);
    v
}

pub fn rlp_encode_numerical(us: &[u8]) -> Vec<u8> {
    rlp_encode_bytes(trim_bytes(us))
}

fn rlp_encode_list(us: &[u8]) -> Vec<u8> {
    let mut v = if us.len() <= 55 {
        vec![0xc0 + (us.len() as u8)]
    } else {
        let len_be = us.len().to_be_bytes();
        let len_bytes = trim_bytes(&len_be);
        let mut v = vec![0xf7 + (len_bytes.len() as u8)];
        v.extend(len_bytes);
        v
    };
    v.extend(us);
    v
}

pub trait RlpEncodable {
    fn to_rlp_bytes(&self) -> Vec<u8>;
}

impl RlpEncodable for u64 {
    fn to_rlp_bytes(&self) -> Vec<u8> {
        rlp_encode_numerical(&self.to_be_bytes())
    }
}

impl<T: RlpEncodable> RlpEncodable for Vec<T> {
    fn to_rlp_bytes(&self) -> Vec<u8> {
        rlp_encode_list(
            &self
                .iter()
                .map(|item| item.to_rlp_bytes())
                .collect::<Vec<Vec<u8>>>()
                .concat(),
        )
    }
}

pub struct RlpStream {
    buf: Vec<u8>,
}

impl RlpStream {
    pub fn new() -> Self {
        Self { buf: vec![] }
    }

    pub fn push(&mut self, item: &dyn RlpEncodable) {
        self.buf.extend(item.to_rlp_bytes());
    }

    pub fn extend(&mut self, slice: &[u8]) {
        self.buf.extend(slice);
    }

    pub fn finalize_without_length_prefix(&self) -> Vec<u8> {
        self.buf.clone()
    }

    pub fn finalize_with_length_prefix(&self) -> Vec<u8> {
        rlp_encode_list(&self.buf)
    }
}
