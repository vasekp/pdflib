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
