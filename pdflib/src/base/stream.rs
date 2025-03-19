use super::*;
use super::types::*;

/// Used to implement a discriminator between by-reference and by-value stored streams.
///
/// This is a marker trait intended to only be implemented for
/// - [`Offset`] (i.e., `u64`), for references into an input file,
/// - `Vec<u8>`, for stream data stored as part of the object itself.
///
/// These two types can then form the `data` field of a [`Stream`].
pub trait StreamData {}
pub(crate) type ByRef = Offset;
pub(crate) type ByVal = Vec<u8>;

impl StreamData for ByRef {}

impl StreamData for ByVal {}

/// A PDF stream object.
///
/// The `Data` parameter may be either [`Offset`] for streams referring to an opened file or 
/// `Vec<u8>` when the data is stored in a detached form.
#[derive(Debug, PartialEq, Clone)]
pub struct Stream<Data: StreamData> {
    /// The stream dictionary.
    pub dict: Dict,
    /// The stream data, or its offset in the file (relative to `%PDF`).
    ///
    /// See [`StreamData`] for more details.
    pub data: Data
}

/// A shorthand for [`Stream<Offset>`].
pub type RefStream = Stream<ByRef>;

/// A shorthand for [`Stream<Vec<u8>>`].
pub type OwnedStream = Stream<ByVal>;
