pub fn parse_num<T: std::str::FromStr>(bstr: &[u8]) -> Option<T> {
    std::str::from_utf8(bstr).ok()?
        .parse::<T>().ok()
}

pub(crate) trait ObjNumGen { }
impl ObjNumGen for crate::base::types::ObjNum { }
impl ObjNumGen for crate::base::types::ObjGen { }

pub fn parse_int_strict<T>(bstr: &[u8]) -> Option<T>
    where T: ObjNumGen + std::str::FromStr
{
    match bstr {
        [b'0'] | [b'1'..=b'9', ..] => parse_num(bstr),
        _ => None
    }
}

pub fn hex_value(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None
    }
}
