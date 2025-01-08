pub fn parse_num<T: std::str::FromStr>(bstr: &[u8]) -> Option<T> {
    std::str::from_utf8(bstr).ok()?
        .parse::<T>().ok()
}

pub fn hex_value(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None
    }
}
