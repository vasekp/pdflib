mod flate;
mod asciihex;

use crate::base::*;
use std::io::BufRead;

/// Supported PDF filters.
pub enum Filter {
    /// `/FlateDecode`
    Flate,
    /// `/ASCIIHexDecode`
    AsciiHex,
}

impl TryFrom<&Name> for Filter {
    type Error = Error;

    fn try_from(name: &Name) -> Result<Filter, Error> {
        if name == b"FlateDecode" {
            Ok(Filter::Flate)
        } else if name == b"ASCIIHexDecode" {
            Ok(Filter::AsciiHex)
        } else {
            Err(Error::Parse("unimplemented filter"))
        }
    }
}

/// Wraps a `BufRead` in an adapter decoding the data according to the provided `/Filter` and 
/// `/DecodeParms` configuration.
///
/// Both of these need to be provided as fully resolved objects. Moreover, the `filter` argument
/// needs to be provided in the form of an array of [`Filter`]s. To convert a generic `Object` 
/// to this unified format, see [`to_filters()`].
pub fn decode<'a, R: BufRead + 'a>(input: R, filter: &[Filter], params: Option<&Dict>) -> Box<dyn BufRead + 'a> {
    match filter {
        [] => Box::new(input),
        [Filter::Flate] => flate::decode(input, params.unwrap_or(&Dict::default())),
        [Filter::AsciiHex] => Box::new(asciihex::decode(input)),
        _ => {
            //error!("skipping unimplemented filter: {:?}", filter);
            //Box::new(input)
            panic!() // TODO
        }
    }
}

/// Resolve a PDF `Object` value of the `/Filter` key into the format expected by [`decode()`].
pub fn to_filters(obj: &Object) -> Result<Vec<Filter>, Error> {
    match obj {
        Object::Name(name) => Ok(vec![name.try_into()?]),
        Object::Array(vec) => vec.iter()
            .map(|obj| match obj {
                Object::Name(name) => name.try_into(),
                _ => Err(Error::Parse("malformed /Filter"))
            })
            .collect::<Result<Vec<_>, _>>(),
        Object::Null => Ok(vec![]),
        _ => Err(Error::Parse("malformed /Filter"))
    }
}
