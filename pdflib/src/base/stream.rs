use super::*;
use super::types::*;

/// A PDF stream object.
#[derive(Debug, PartialEq, Clone)]
pub struct Stream {
    /// The stream dictionary.
    pub dict: Dict,
    /// The stream data. A reference or the full content may be stored. See [`Data`] for details.
    pub data: Data
}

/// The data type of [`Stream::data`].
#[derive(Debug, PartialEq, Clone)]
pub enum Data {
    /// Reference to a file, given by offset from `%PDF`.
    ///
    /// NB that length is part of the stream dictionary ([`Stream::dict`]) and may be stored as an 
    /// indirect object, potentionally even missing or wrong.
    Ref(Offset),
    /// The actual content, unfiltered, verbatim.
    Val(Vec<u8>)
}
