#[derive(Debug)]
pub enum Error {
    IO(std::io::Error),
    Parse(&'static str)
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Error {
        Error::IO(err)
    }
}
