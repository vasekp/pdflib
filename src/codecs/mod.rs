mod flate;
mod asciihex;

use crate::base::*;
use std::io::BufRead;

pub fn decode<'a, R: BufRead + 'a>(input: R, filter: &[Name]) -> Box<dyn BufRead + 'a> {
    match filter {
        [] => Box::new(input),
        [name] if name == b"FlateDecode" => Box::new(flate::decode(input)),
        [name] if name == b"ASCIIHexDecode" => Box::new(asciihex::decode(input)),
        _ => unimplemented!("codec: {:?}", filter)
    }
}

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
