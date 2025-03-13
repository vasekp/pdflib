mod flate;
mod asciihex;

use crate::base::*;
use std::io::BufRead;

/// Supported PDF filters.
#[derive(Debug, PartialEq)]
pub enum Filter {
    /// `/FlateDecode`
    Flate,
    /// `/ASCIIHexDecode`
    AsciiHex,
}

impl TryFrom<&Name> for Filter {
    type Error = Error;

    fn try_from(name: &Name) -> Result<Filter, Error> {
        use std::ops::Deref;
        match name.deref() {
            b"FlateDecode" => Ok(Filter::Flate),
            b"ASCIIHexDecode" => Ok(Filter::AsciiHex),
            _ => Err(Error::Parse("unimplemented filter"))
        }
    }
}

/// Wraps a `BufRead` in an adapter decoding the data according to the provided `/Filter` and 
/// `/DecodeParms` configuration.
///
/// The latter needs to be provided as fully resolved objects. Moreover, the `filter` argument 
/// needs to be provided in the form of an array of [`Filter`]s.
pub fn decode<'a, R: BufRead + 'a>(input: R, filter: &[Filter], params: Option<&Dict>) -> Box<dyn 
BufRead + 'a> {
    match filter {
        [] => Box::new(input),
        [Filter::Flate] => flate::decode(input, params.unwrap_or(&Dict::default())),
        [Filter::AsciiHex] => Box::new(asciihex::decode(input)),
        _ => panic!() // TODO
    }
}
