use super::types::*;

/// Encodes information about the file header.
///
/// Available through [`parser::FileParser::header()`](crate::parser::FileParser::header).
#[derive(Debug)]
pub struct Header {
    /// The byte offset of the `%PDF` header from start of file data.
    pub start: Offset,
    /// Version (major, minor).
    pub version: (u8, u8),
}
