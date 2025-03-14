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
pub fn decode<'a, R: BufRead + 'a>(input: R, filter: &[Filter], params: Option<&Dict>) -> Box<dyn BufRead + 'a> {
    match filter {
        [] => Box::new(input),
        [Filter::Flate] => flate::decode(input, params.unwrap_or(&Dict::default())),
        [Filter::AsciiHex] => Box::new(asciihex::decode(input)),
        [_, ..] => decode(decode(input, &filter[..1], params), &filter[1..], None),
    }
}

pub fn parse_filters(dict: &Dict, res: &impl Resolver) -> Result<Vec<Filter>, Error> {
    let obj = dict.lookup(b"Filter");
    let binding;
    let obj_res = match obj {
        Object::Ref(objref) => {
            binding = res.resolve_ref(objref)?;
            &binding
        },
        _ => obj
    };
    match obj_res {
        Object::Name(name) => Ok(vec![name.try_into()?]),
        Object::Array(vec) => {
            let mut ret = Vec::new();
            for item in vec {
                let binding;
                let item_res = match item {
                    Object::Ref(objref) => {
                        binding = res.resolve_ref(objref)?;
                        &binding
                    },
                    _ => item
                };
                let filter = item_res.as_name()
                    .ok_or(Error::Parse("malformed /Filter"))?
                    .try_into()?;
                ret.push(filter);
            }
            Ok(ret)
        },
        Object::Null => Ok(vec![]),
        _ => Err(Error::Parse("malformed /Filter"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_filter_chaining() {
        let data = "78 9c 2b 49 2d 2e 01 00 04 5d 01 c1";
        let input = Cursor::new(data);
        let mut output = decode(input, &[Filter::AsciiHex, Filter::Flate], None);
        let mut data_out = String::new();
        output.read_to_string(&mut data_out).unwrap();
        assert_eq!(data_out, "test");
    }
}
