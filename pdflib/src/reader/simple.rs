use std::io::{BufRead, Seek};

use crate::base::*;
use crate::base::types::*;
use crate::parser::FileParser;

use super::base::BaseReader;

/// Allows finding and parsing objects in a PDF file through the cross-reference table.
///
/// `SimpleReader` is best suited for well-formed files with a complete and undamaged xref table 
/// and is designed for performance. Upon construction, it builds the complete xref table from its 
/// sections for a quick lookup. Object streams are also cached for better performance. Other kinds 
/// of caching are left to the user.
pub struct SimpleReader<T: BufRead + Seek> {
    base: BaseReader<T>,
    pub xref: XRef,
}

impl<T: BufRead + Seek> SimpleReader<T> {
    /// Creates a `SimpleReader` instance around a `BufRead + Seek` source.
    ///
    /// Returns with an error if the cross-reference table is not found or damaged.
    pub fn new(source: T) -> Result<Self, Error> {
        let parser = FileParser::new(source);
        let entry = parser.entrypoint()?;
        let xref = Self::build_xref(&parser, entry)?;
        let base = BaseReader::new(parser);
        Ok(Self { base, xref })
    }

    fn build_xref(parser: &FileParser<T>, entry: Offset) -> Result<XRef, Error> {
        let mut iter = BaseReader::read_xref_chain(parser, entry);
        let mut order = vec![entry];
        let mut xref = iter.next().ok_or(Error::Parse("could not parse xref table"))?.1;
        for (offset, next_xref) in iter {
            if order.contains(&offset) {
                log::warn!("Breaking xref chain detected at {offset}.");
                break;
            }
            xref.merge_prev(next_xref);
            order.push(offset);
        }
        Ok(xref)
    }

    /// Iterates over all object numbers marked as used, in increasing number.
    ///
    /// Each object is parsed at the moment of retrieval, which can result in an [`Error`]. Such 
    /// errors usually have no consequences for the subsequent objects, so the iterator can be used 
    /// further.
    pub fn objects(&self) -> impl Iterator<Item = (ObjRef, Result<Object, Error>)> + '_ {
        self.xref.map.iter()
            .flat_map(move |(&num, rec)| match *rec {
                Record::Used{gen, offset} => {
                    let objref = ObjRef{num, gen};
                    Some((objref, self.base.read_uncompressed(offset, &objref)))
                },
                Record::Compr{num_within, index} => {
                    let objref = ObjRef{num, gen: 0};
                    Some((objref, self.base.read_compressed(num_within, index, &self.xref, &objref)))
                },
                Record::Free{..} => None
            })
    }

    /// Resolves an [`ObjRef`] into an owned [`Object`].
    pub fn resolve_ref(&self, objref: &ObjRef) -> Result<Object, Error> {
        self.base.resolve_ref(objref, &self.xref)
    }

    /// For an [`Object::Ref`], calls [`SimpleReader::resolve_ref()`], otherwise returns `obj` 
    /// unchanged.
    pub fn resolve_obj(&self, obj: Object) -> Result<Object, Error> {
        self.base.resolve_obj(obj, &self.xref)
    }

    /// Resolves indirect references like [`SimpleReader::resolve_obj()`], but also traverses to 
    /// the first level in [`Object::Array`]s and [`Object::Dict`]s.
    pub fn resolve_deep(&self, obj: Object) -> Result<Object, Error> {
        self.base.resolve_deep(obj, &self.xref)
    }

    /// Creates a `BufRead` reading stream data for a [`Stream`], after decoding using the values 
    /// of `/Filter` and `/DecodeParms` from the stream dictionary.
    ///
    /// If the length can not be determined (e.g. the `/Length` entry refers to a missing object), 
    /// the data is read until the first occurrence of the `endstream` keyword. A warning is 
    /// emitted in such case.
    ///
    /// If the filter (or one of the filters) are not implemented, a warning is also emitted and 
    /// the data is returned in its original encoded form.
    ///
    /// Note that this is a mutable borrow of an internal `RefCell`, so in order to prevent runtime 
    /// borrow checking failures, you may need to manually `drop()` the instance prior to calling 
    /// any other methods of this `SimpleReader`.
    pub fn read_stream_data(&self, obj: &Stream) -> Result<Box<dyn BufRead + '_>, Error> {
        self.base.read_stream_data(obj, &self.xref)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::io::*;
    use std::fs::*;
    use crate::parser::bp::ByteProvider;

    #[test]
    fn test_objects_iter() {
        let rdr = SimpleReader::new(BufReader::new(File::open("src/tests/basic.pdf").unwrap())).unwrap();
        let mut iter = rdr.objects();

        let (oref, res) = iter.next().unwrap();
        let obj = res.unwrap();
        assert_eq!(oref, ObjRef { num: 1, gen: 0 });
        assert_eq!(obj, Object::Dict(Dict::from(vec![
            (Name::from(b"Type"), Object::new_name(b"Pages")),
            (Name::from(b"Kids"), Object::Array(vec![Object::Ref(ObjRef { num: 2, gen: 0 })])),
            (Name::from(b"Count"), Object::Number(Number::Int(1))),
        ])));
        let kids = rdr.resolve_ref(&ObjRef { num: 2, gen: 0 }).unwrap();

        let (oref, res) = iter.next().unwrap();
        let obj = res.unwrap();
        assert_eq!(oref, ObjRef { num: 2, gen: 0 });
        assert_eq!(obj, kids);

        let mut iter = iter.skip(1);

        let (oref, res) = iter.next().unwrap();
        assert_eq!(oref, ObjRef { num: 4, gen: 0 });
        let stm = res.unwrap().into_stream().unwrap();
        let mut data = rdr.read_stream_data(&stm).unwrap();
        let line = data.read_line_excl().unwrap();
        assert_eq!(line, b"1 0 0 -1 0 841.889771 cm");

        //etc.
    }

    #[test]
    fn test_xref_chaining() {
        let rdr = SimpleReader::new(BufReader::new(File::open("src/tests/hybrid.pdf").unwrap())).unwrap();
        let stm = rdr.resolve_ref(&ObjRef { num: 4, gen: 0 })
            .unwrap()
            .into_stream()
            .unwrap();
        let mut data = rdr.read_stream_data(&stm).unwrap();
        let mut s = Vec::new();
        data.read_to_end(&mut s).unwrap();
        assert_eq!(s, b"BT /F1 12 Tf 72 720 Td (Hello, update!) Tj ET");

        let rdr = SimpleReader::new(BufReader::new(File::open("src/tests/updates.pdf").unwrap())).unwrap();
        let stm = rdr.resolve_ref(&ObjRef { num: 1, gen: 0 })
            .unwrap()
            .into_stream()
            .unwrap();
        let mut data = rdr.read_stream_data(&stm).unwrap();
        let mut s = Vec::new();
        data.read_to_end(&mut s).unwrap();
        assert_eq!(s, b"Test with diff length");

        let rdr = SimpleReader::new(BufReader::new(File::open("src/tests/circular.pdf").unwrap())).unwrap();
        assert!(rdr.xref.map.is_empty());
    }
}
