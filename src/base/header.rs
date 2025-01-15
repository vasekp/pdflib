use super::types::*;

#[derive(Debug)]
pub struct Header {
    pub start: Offset,
    pub version: (u8, u8),
    // TODO pub binary_chars: Vec<u8>
}
