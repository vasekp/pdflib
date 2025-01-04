mod flate;

use crate::base::*;
use std::io::BufRead;

pub fn decode<'a, R: BufRead + 'a>(input: R, filter: &Object) -> Box<dyn BufRead + 'a> {
    match filter {
        Object::Null => Box::new(input),
        Object::Name(n) if n == b"FlateDecode" => Box::new(flate::decode(input)),
        _ => unimplemented!()
    }
}
