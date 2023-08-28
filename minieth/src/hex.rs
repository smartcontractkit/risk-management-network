pub fn decode(s: &str) -> Option<Vec<u8>> {
    let hex = s.strip_prefix("0x").unwrap_or(s);
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let mut res = Vec::with_capacity((hex.len() + 1) / 2);
    let mut from = 0;
    let mut to = if hex.len() % 2 == 1 {
        from + 1
    } else {
        from + 2
    };
    while from < hex.len() {
        res.push(u8::from_str_radix(&hex[from..to], 16).ok()?);
        from = to;
        to = from + 2;
    }
    Some(res)
}

pub fn encode(us: &[u8]) -> String {
    let mut s = String::with_capacity(2 + us.len() * 2);
    s.push_str("0x");
    for u in us {
        s.push_str(&format!("{u:02x}"));
    }
    s
}
