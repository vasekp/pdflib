use std::fmt::{Display, Formatter};

/// The common error type of this library.
///
/// Currently, this holds either a [`std::io::Error`] or a static string.
///
/// The [`Error::IO`] case is held via a [`std::rc::Rc`] in order for instances to be clone-able.
#[derive(Debug, Clone)]
pub enum Error {
    IO(std::rc::Rc<std::io::Error>),
    Parse(&'static str)
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Error {
        Error::IO(std::rc::Rc::new(err))
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::IO(io) => write!(f, "IO: {io}"),
            Error::Parse(err) => write!(f, "{err}")
        }
    }
}
