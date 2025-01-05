mod flate;
mod asciihex;

use crate::base::*;
use std::io::Read;

pub fn decode<'a, R: Read + 'a>(input: R, filter: &Object) -> Box<dyn Read + 'a> {
    match filter {
        Object::Null => Box::new(input),
        Object::Name(n) if n == b"FlateDecode" => Box::new(flate::decode(input)),
        Object::Name(n) if n == b"ASCIIHexDecode" => Box::new(asciihex::decode(input)),
        _ => unimplemented!()
    }
}
