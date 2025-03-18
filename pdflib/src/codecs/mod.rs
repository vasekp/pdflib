mod flate;
mod asciihex;
mod ascii85;

use crate::base::*;
use std::io::BufRead;

/// Supported PDF filters.
#[derive(Debug, PartialEq)]
pub enum Filter {
    /// `/FlateDecode` (supporting `/DecodeParms`).
    Flate(Dict),
    /// `/ASCIIHexDecode`
    AsciiHex,
    /// `ASCII85Decode`
    Ascii85,
}

impl Filter {
    fn try_from(name: &Name, params: Option<Dict>) -> Result<Filter, Error> {
        match name.as_slice() {
            b"FlateDecode" => Ok(Filter::Flate(params.unwrap_or_default())),
            b"ASCIIHexDecode" => {
                if params.is_some() {
                    log::warn!("Ingoring /DecodeParms for /ASCIIHexDecode.");
                }
                Ok(Filter::AsciiHex)
            },
            b"ASCII85Decode" => {
                if params.is_some() {
                    log::warn!("Ingoring /DecodeParms for /ASCIIHexDecode.");
                }
                Ok(Filter::Ascii85)
            },
            _ => Err(Error::Parse("unimplemented filter"))
        }
    }
}

/// Wraps a `BufRead` in an adapter decoding the data according to the provided filter chain.
pub fn decode<'a, R: BufRead + 'a>(input: R, filter: &[Filter]) -> Box<dyn BufRead + 'a> {
    match filter {
        [] => Box::new(input),
        [Filter::Flate(params)] => flate::decode(input, params),
        [Filter::AsciiHex] => Box::new(asciihex::decode(input)),
        [Filter::Ascii85] => Box::new(ascii85::decode(input)),
        [_, ..] => decode(decode(input, &filter[..1]), &filter[1..]),
    }
}

/// Parse stream dictionary's `/Filter` and `/DecodeParms` entries into the form expected by 
/// [`codecs::decode`](decode).
pub fn parse_filters(dict: &Dict, res: &impl Resolver) -> Result<Vec<Filter>, Error> {
    let filter = dict.lookup(b"Filter");
    let params = dict.lookup(b"DecodeParms").to_owned();
    let binding;
    let filter_res = match filter {
        Object::Ref(objref) => {
            binding = res.resolve_ref(objref)?;
            &binding
        },
        _ => filter
    };
    match filter_res {
        Object::Name(name) => {
            let params = match params {
                Object::Dict(dict) => Some(dict),
                Object::Null => None,
                _ => return Err(Error::Parse("malformed /DecodeParms"))
            };
            Ok(vec![Filter::try_from(name, params)?])
        },
        Object::Array(filters) => {
            let params = match params {
                Object::Null => None,
                Object::Array(arr) => {
                    if arr.len() != filters.len() {
                        return Err(Error::Parse("malformed /DecodeParms"));
                    }
                    Some(arr)
                },
                _ => return Err(Error::Parse("malformed /DecodeParms"))
            };
            let mut ret = Vec::new();
            let mut params_iter = params.map(IntoIterator::into_iter);
            for item in filters {
                let binding;
                let item_res = match item {
                    Object::Ref(objref) => {
                        binding = res.resolve_ref(objref)?;
                        &binding
                    },
                    _ => item
                };
                let params = match params_iter.as_mut().and_then(Iterator::next) {
                    Some(Object::Dict(dict)) => Some(dict),
                    None | Some(Object::Null) => None,
                    _ => return Err(Error::Parse("malformed /DecodeParms"))
                };
                let filter = Filter::try_from(item_res.as_name()
                    .ok_or(Error::Parse("malformed /Filter"))?, params)?;
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
        let mut output = decode(input, &[Filter::AsciiHex, Filter::Flate(Dict::default())]);
        let mut data_out = String::new();
        output.read_to_string(&mut data_out).unwrap();
        assert_eq!(data_out, "test");
    }
}
