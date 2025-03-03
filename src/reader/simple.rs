use std::io::{BufRead, Seek};

use crate::base::*;
use crate::base::types::*;
use crate::parser::FileParser;

use super::base::BaseReader;

pub struct SimpleReader<T: BufRead + Seek> {
    base: BaseReader<T>,
    xref: XRef,
}

impl<T: BufRead + Seek> SimpleReader<T> {
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

    pub fn resolve_ref(&self, objref: &ObjRef) -> Result<Object, Error> {
        self.base.resolve_ref(objref, &self.xref)
    }

    pub fn resolve_obj(&self, obj: &Object) -> Result<Object, Error> {
        self.base.resolve_obj(obj, &self.xref)
    }

    pub fn resolve_deep(&self, obj: &Object) -> Result<Object, Error> {
        self.base.resolve_deep(obj, &self.xref)
    }

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
        assert_eq!(obj, Object::Dict(Dict(vec![
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
        let obj = res.unwrap();
        let Object::Stream(stm) = obj else { panic!() };
        let mut data = rdr.read_stream_data(&stm).unwrap();
        let line = data.read_line_excl().unwrap();
        assert_eq!(line, b"1 0 0 -1 0 841.889771 cm");

        //etc.
    }

    #[test]
    fn test_resolve_deep() {
        let rdr = SimpleReader::new(BufReader::new(File::open("src/tests/indirect-filters.pdf").unwrap())).unwrap();
        let obj = rdr.resolve_ref(&ObjRef { num: 4, gen: 0 }).unwrap();
        let Object::Stream(Stream { dict, .. }) = obj else { panic!() };
        let fil = dict.lookup(b"Filter");
        let res = rdr.resolve_deep(&fil).unwrap();
        assert_eq!(res, Object::Array(vec![ Object::new_name(b"AsciiHexDecode"), Object::new_name(b"FlateDecode")]));
    }

    /*#[test]
    fn test_xref_chaining() {
        let rdr = SimpleReader::new(BufReader::new(File::open("src/tests/hybrid.pdf").unwrap()));
        assert_eq!(rdr.entry, Some(912));
        let x912 = rdr.xrefs.get(&912).unwrap();
        let x759 = rdr.xrefs.get(&759).unwrap();
        let x417 = rdr.xrefs.get(&417).unwrap();
        assert_eq!(rdr.base_locator() as *const dyn Locator as *const (),
            x912 as &dyn Locator as *const dyn Locator as *const ());
        // main table's /XRefStm
        assert_eq!(Rc::as_ptr(x912.next.as_ref().unwrap()), Rc::as_ptr(&x759));
        // main table's /Prev
        assert_eq!(Rc::as_ptr(x759.next.as_ref().unwrap()), Rc::as_ptr(&x417));
        assert!(x417.next.is_none());

        // 912 itself does not define 4 0
        assert_eq!(x912.curr.locate(&ObjRef { num: 4, gen: 0 }), None);
        // but should continue looking down the chain
        assert_eq!(x912.locate(&ObjRef { num: 4, gen: 0 }), Some(Record::Used { gen: 0, offset: 644 }));
        // 759 does, updating older definition
        assert_eq!(x759.locate(&ObjRef { num: 4, gen: 0 }), Some(Record::Used { gen: 0, offset: 644 }));
        // 417's definition is different and shadowed
        assert_eq!(x417.locate(&ObjRef { num: 4, gen: 0 }), Some(Record::Used { gen: 0, offset: 251 }));

        // 912 defines 6 0
        assert_eq!(x912.locate(&ObjRef { num: 6, gen: 0 }), Some(Record::Used { gen: 0, offset: 759 }));
        // 759 does not, though it fits in its /Size
        assert_eq!(x759.curr.locate(&ObjRef { num: 6, gen: 0 }), None);
        assert_eq!(x759.locate(&ObjRef { num: 6, gen: 0 }), Some(Record::default()));
        // 417 has smaller /Size so it should reject right away
        assert_eq!(x417.curr.locate(&ObjRef { num: 6, gen: 0 }), Some(Record::default()));
        assert_eq!(x417.locate(&ObjRef { num: 6, gen: 0 }), Some(Record::default()));

        let rdr = SimpleReader::new(BufReader::new(File::open("src/tests/updates.pdf").unwrap()));
        assert_eq!(rdr.entry, Some(510));
        let x510 = rdr.xrefs.get(&510).unwrap();
        let x322 = rdr.xrefs.get(&322).unwrap();
        let x87 = rdr.xrefs.get(&87).unwrap();
        // main table's /Prev
        assert_eq!(Rc::as_ptr(x510.next.as_ref().unwrap()), Rc::as_ptr(&x322));
        // /Prev's /Prev
        assert_eq!(Rc::as_ptr(x322.next.as_ref().unwrap()), Rc::as_ptr(&x87));
        assert!(x87.next.is_none());

        let Object::Stream(stm) = rdr.resolve_ref(&ObjRef { num: 1, gen: 0 }, x87).unwrap()
            else { panic!() };
        let mut data = rdr.read_stream_data(&stm, x87).unwrap();
        let mut s = String::new();
        data.read_to_string(&mut s).unwrap();
        drop(data);
        assert_eq!(s, "Test 1");

        let Object::Stream(stm) = rdr.resolve_ref(&ObjRef { num: 1, gen: 0 }, x322).unwrap()
            else { panic!() };
        let mut data = rdr.read_stream_data(&stm, x322).unwrap();
        let mut s = String::new();
        data.read_to_string(&mut s).unwrap();
        drop(data);
        assert_eq!(s, "Test 2");

        let Object::Stream(stm) = rdr.resolve_ref(&ObjRef { num: 1, gen: 0 }, x510).unwrap()
            else { panic!() };
        let mut data = rdr.read_stream_data(&stm, x510).unwrap();
        let mut s = String::new();
        data.read_to_string(&mut s).unwrap();
        drop(data);
        assert_eq!(s, "Test with diff length");

        let rdr = SimpleReader::new(BufReader::new(File::open("src/tests/circular.pdf").unwrap()));
        assert_eq!(rdr.entry, Some(9));
        let x9 = rdr.xrefs.get(&9).unwrap();
        // illegal /Prev (leading to itself)
        assert_eq!(x9.curr.dict.lookup(b"Prev"), &Object::Number(Number::Int(9)));
        // not propagated into the linked list
        assert!(x9.next.is_none());
    }*/

    /*#[test]
    fn test_objstm_caching() {
        let rdr = SimpleReader::new(BufReader::new(File::open("src/tests/objstm.pdf").unwrap())).unwrap();
        assert_eq!(rdr.xref.locate(&ObjRef { num: 1, gen: 0 }), Some(Record::Compr { num_within: 8, index: 4 }));
        assert!(rdr.objstms.borrow().is_empty());
        let obj = rdr.resolve_ref(&ObjRef { num: 1, gen: 0 }).unwrap();
        assert_eq!(obj, Object::Dict(Dict(vec![
            (Name::from(b"Pages"), Object::Ref(ObjRef { num: 9, gen: 0 })),
            (Name::from(b"Type"), Object::new_name(b"Catalog")),
        ])));

        let objstms = rdr.objstms.borrow();
        assert!(!objstms.is_empty());
        let line = objstms.get(&4973).unwrap().as_ref().unwrap().source.as_slice().read_line_excl().unwrap();
        assert_eq!(line, b"<</Font<</F1 5 0 R>>/ProcSet[/PDF/Text/ImageC/ImageB/ImageI]>>");
        drop(objstms);

        let obj2 = rdr.resolve_ref(&ObjRef { num: 1, gen: 0 }).unwrap();
        assert_eq!(obj, obj2);
    }*/

    /*#[test]
    fn test_read_objstm_take() {
        let source = "1 0 obj <</Type/ObjStm /N 3 /First 11 /Length 14>> stream
2 0 3 1 4 2614endstream endobj";
        let rdr = SimpleReader {
            parser: FileParser::new(Cursor::new(source)),
            xref: null_xref(),
            objstms: Default::default(),
        };
        let objstm = rdr.read_objstm(0, &ObjRef { num: 1, gen: 0 }).unwrap();
        assert_eq!(objstm.entries, vec![(2, 0), (3, 1), (4, 2)]);
        assert_eq!(objstm.source, b"614");
        assert_eq!(rdr.resolve_ref(&ObjRef { num: 2, gen: 0 }).unwrap(),
            Object::Number(Number::Int(6)));
        assert_eq!(rdr.resolve_ref(&ObjRef { num: 3, gen: 0 }).unwrap(),
            Object::Number(Number::Int(1)));
        assert_eq!(rdr.resolve_ref(&ObjRef { num: 4, gen: 0 }).unwrap(),
            Object::Number(Number::Int(4)));
    }*/

    /*#[test]
    fn test_read_stream_overflow() {
        let source = "1 0 obj <</Length 10>> stream\n123\nendstream endobj";
        let rdr = SimpleReader::new(Cursor::new(source));
        let Object::Stream(stm) = rdr.read_uncompressed(0, &ObjRef { num: 1, gen: 0 }).unwrap()
            else { panic!() };
        let mut data = rdr.read_stream_data(&stm, &()).unwrap();
        let mut s = String::new();
        data.read_to_string(&mut s).unwrap();
        drop(data);
        assert_eq!(s, "123\nendstr");

        let source = "1 0 obj <</Length 100>> stream\n123\nendstream endobj";
        let rdr = SimpleReader::new(Cursor::new(source));
        let Object::Stream(stm) = rdr.read_uncompressed(0, &ObjRef { num: 1, gen: 0 }).unwrap()
            else { panic!() };
        let mut data = rdr.read_stream_data(&stm, &()).unwrap();
        let mut s = String::new();
        data.read_to_string(&mut s).unwrap();
        drop(data);
        assert_eq!(s, "123\nendstream endobj");

        let source = "1 0 obj <</Length 10>> stream\n123";
        let rdr = SimpleReader::new(Cursor::new(source));
        let Object::Stream(stm) = rdr.read_uncompressed(0, &ObjRef { num: 1, gen: 0 }).unwrap()
            else { panic!() };
        let mut data = rdr.read_stream_data(&stm, &()).unwrap();
        let mut s = String::new();
        data.read_to_string(&mut s).unwrap();
        drop(data);
        assert_eq!(s, "123");

        let source = "1 0 obj <<>> stream\n123\n45endstream endobj";
        let rdr = SimpleReader::new(Cursor::new(source));
        let Object::Stream(stm) = rdr.read_uncompressed(0, &ObjRef { num: 1, gen: 0 }).unwrap()
            else { panic!() };
        let mut data = rdr.read_stream_data(&stm, &()).unwrap();
        let mut s = String::new();
        data.read_to_string(&mut s).unwrap();
        drop(data);
        assert_eq!(s, "123\n45");

        let source = "1 0 obj <<>> stream\n123";
        let rdr = SimpleReader::new(Cursor::new(source));
        let Object::Stream(stm) = rdr.read_uncompressed(0, &ObjRef { num: 1, gen: 0 }).unwrap()
            else { panic!() };
        let mut data = rdr.read_stream_data(&stm, &()).unwrap();
        let mut s = String::new();
        data.read_to_string(&mut s).unwrap();
        drop(data);
        assert_eq!(s, "123");
    }*/
}
