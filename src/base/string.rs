use std::fmt::Formatter;

//TODO: literal / hex heuristics
pub(crate) fn format_string(f: &mut Formatter<'_>, s: &Vec<u8>) -> std::fmt::Result {
    f.write_str("(")?;
    for c in s {
        match c {
            b'\x0a' => f.write_str("\\n"),
            b'\x0d' => f.write_str("\\r"),
            b'\x09' => f.write_str("\\t"),
            b'\x08' => f.write_str("\\b"),
            b'\x0c' => f.write_str("\\f"),
            b'(' => f.write_str("\\("),
            b')' => f.write_str("\\)"),
            b'\\' => f.write_str("\\\\"),
            b'\x20' ..= b'\x7E' => write!(f, "{}", *c as char),
            _ => write!(f, "\\{c:03o}")
        }?
    }
    f.write_str(")")
}

