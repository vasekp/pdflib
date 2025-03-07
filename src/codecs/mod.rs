mod flate;
mod asciihex;

use crate::base::*;
use std::io::BufRead;
use log::*;

/// Wraps a `BufRead` in an adapter decoding the data according to the provided `/Filter` and 
/// `/DecodeParms` configuration.
///
/// Both of these need to be provided as fully resolved objects. Moreover, the `filter` argument
/// needs to be provided in the form of an array of [`Name`]s. To convert a generic `Object` (which 
/// conformantly also could be a single `/Name` or `null`) to this unified format, see 
/// [`to_filters()`].
pub fn decode<'a, R: BufRead + 'a>(input: R, filter: &[Name], params: Option<&Dict>) -> Box<dyn BufRead + 'a> {
    match filter {
        [] => Box::new(input),
        [name] if name == b"FlateDecode" => flate::decode(input, params.unwrap_or(&Dict::default())),
        [name] if name == b"ASCIIHexDecode" => Box::new(asciihex::decode(input)),
        _ => {
            error!("skipping unimplemented filter: {:?}", filter);
            Box::new(input)
        }
    }
}

/// Resolve a PDF `Object` value of the `/Filter` key into the format expected by [`decode()`].
pub fn to_filters(obj: &Object) -> Result<Vec<Name>, Error> {
    match obj {
        Object::Name(name) => Ok(vec![name.to_owned()]),
        Object::Array(vec) => vec.iter()
            .map(|obj| match obj {
                Object::Name(name) => Ok(name.to_owned()),
                _ => Err(Error::Parse("malformed /Filter"))
            })
            .collect::<Result<Vec<_>, _>>(),
        Object::Null => Ok(vec![]),
        _ => Err(Error::Parse("malformed /Filter"))
    }
}
