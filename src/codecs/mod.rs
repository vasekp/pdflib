mod flate;
mod asciihex;

use crate::base::*;
use std::io::Read;

pub fn decode<'a, R: Read + 'a>(input: R, filter: &[Name]) -> Box<dyn Read + 'a> {
    match &filter[..] {
        [] => Box::new(input),
        [name] if name == b"FlateDecode" => Box::new(flate::decode(input)),
        [name] if name == b"ASCIIHexDecode" => Box::new(asciihex::decode(input)),
        _ => unimplemented!()
    }
}
