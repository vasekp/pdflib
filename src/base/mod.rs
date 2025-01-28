pub mod types {
    pub type ObjNum = u64;
    pub type ObjGen = u16;
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
pub use string::*;

mod xref;
pub use xref::*;

mod locator;
pub use locator::*;

mod error;
pub use error::*;

mod header;
pub use header::*;
