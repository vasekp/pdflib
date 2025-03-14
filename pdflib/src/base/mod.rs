pub mod types {
    /// Object number: type alias for `u64`.
    pub type ObjNum = u64;
    /// Object generation: type alias for `u16`.
    ///
    /// NB that PDF 1.5 technically allows generation numbers larger `u16::MAX` via xref streams,
    /// no real-world case where that would be used is likely. Thus objects numbers not fitting into
    /// u16 are not supported and encountering them results in a runtime error.
    pub type ObjGen = u16;
    /// Index within an object stream: type alias for `u16`.
    ///
    /// The same restriction applies as stated for [`ObjGen`]. NB that it is a good practice to keep 
    /// the counts of objects in an individual object stream low, the specification recommends a limit 
    /// of 100.
    pub type ObjIndex = ObjGen;
    /// Offset within a file (relative to the `%PDF` marker): type alias for `u64`.
    pub type Offset = u64;
}

mod name;
pub use name::*;

mod number;
pub use number::*;

mod dict;
pub use dict::*;

mod object;
pub use object::*;

mod stream;
pub use stream::*;

mod string;

mod xref;
pub use xref::*;

mod locator;
pub use locator::*;

mod resolver;
pub use resolver::*;

mod error;
pub use error::*;

mod header;
pub use header::*;
