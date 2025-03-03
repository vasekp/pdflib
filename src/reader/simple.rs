use std::io::{Read, BufRead, Seek};
use std::collections::{BTreeMap, VecDeque};
use std::collections::btree_map::Entry;
use std::rc::Rc;
use std::cell::{RefCell, Ref};
use std::ops::Deref;

use crate::base::*;
use crate::base::types::*;
use crate::parser::{FileParser, ObjParser};
use crate::codecs;
use crate::utils;

use super::esr::EndstreamReader;

pub struct SimpleReader<T: BufRead + Seek> {
    parser: FileParser<T>,
    xref: XRef,
    objstms: RefCell<BTreeMap<Offset, Result<ObjStm, Error>>>,
}

struct XRefLink {
    curr: XRef,
    next: Option<Rc<XRefLink>>
}

struct ObjStm {
    entries: Vec<(ObjNum, Offset)>,
    source: Vec<u8>,
}

impl<T: BufRead + Seek> SimpleReader<T> {
    pub fn new(source: T) -> Result<Self, Error> {
        let parser = FileParser::new(source);
        let entry = parser.entrypoint()?;
        let xref = Self::build_xref(&parser, entry)?;
        Ok(Self { parser, xref, objstms: Default::default() })
    }

    fn build_xref(parser: &FileParser<T>, entry: Offset) -> Result<XRef, Error> {
        let mut queue = VecDeque::new();
        let mut order = Vec::new();
        let mut xref = parser.read_xref_at(entry)?;
        if matches!(xref.tpe, XRefType::Table) {
            if let Some(offset) = xref.dict.lookup(b"XRefStm").num_value() {
                queue.push_back((offset, true));
            }
        }
        if let Some(offset) = xref.dict.lookup(b"Prev").num_value() {
            queue.push_back((offset, false));
        }
        while let Some((offset, is_aside)) = queue.pop_front() {
            if order.contains(&offset) {
                log::warn!("Breaking xref chain detected at {offset}.");
                break;
            }
            let curr_xref = parser.read_xref_at(offset)?;
            if matches!(curr_xref.tpe, XRefType::Table) {
                if let Some(offset) = curr_xref.dict.lookup(b"XRefStm").num_value() {
                    if !is_aside {
                        queue.push_back((offset, true));
                    } else {
                        log::warn!("/XRefStm pointed to a classical section.");
                    }
                }
            }
            if let Some(offset) = curr_xref.dict.lookup(b"Prev").num_value() {
                if !is_aside {
                    queue.push_back((offset, false));
                } else {
                    log::warn!("Ignoring /Prev in a /XRefStm.");
                }
            }
            xref.merge_prev(curr_xref);
            order.push(offset);
        }
        Ok(xref)
    }

    pub fn objects(&self) -> impl Iterator<Item = (ObjRef, Result<Object, Error>)> + '_ {
        self.xref.map.iter()
            .flat_map(move |(&num, rec)| match *rec {
                Record::Used{gen, offset} => {
                    let objref = ObjRef{num, gen};
                    Some((objref, self.read_uncompressed(offset, &objref)))
                },
                Record::Compr{num_within, index} => {
                    let objref = ObjRef{num, gen: 0};
                    Some((objref, self.read_compressed(num_within, index, &objref)))
                },
                Record::Free{..} => None
            })
    }

    pub fn resolve_ref(&self, objref: &ObjRef) -> Result<Object, Error> {
        match self.xref.locate(objref) {
            Some(Record::Used { offset, .. }) => self.read_uncompressed(offset, objref),
            Some(Record::Compr { num_within, index }) => self.read_compressed(num_within, index, objref),
            _ => Ok(Object::Null)
        }
    }

    pub fn resolve_obj(&self, obj: &Object) -> Result<Object, Error> {
        match obj {
            Object::Ref(objref) => self.resolve_ref(objref),
            _ => Ok(obj.to_owned())
        }
    }

    fn read_uncompressed(&self, offset: Offset, oref_expd: &ObjRef) -> Result<Object, Error> {
        let (oref, obj) = self.parser.read_obj_at(offset)?;
        if &oref == oref_expd {
            Ok(obj)
        } else {
            Err(Error::Parse("object number mismatch"))
        }
    }

    fn read_compressed(&self, num_within: ObjNum, index: ObjIndex, oref_expd: &ObjRef) -> Result<Object, Error> {
        let index = index as usize;
        let cache_ref = self.read_cache_objstm(num_within);
        let objstm = match (*cache_ref).deref() {
            Ok(objstm) => objstm,
            Err(err) => return Err(err.clone())
        };
        let Some(&(num, start_offset)) = objstm.entries.get(index) else {
            return Err(Error::Parse("out of bounds index requested from object stream"));
        };
        if &(ObjRef { num, gen: 0 }) != oref_expd {
            return Err(Error::Parse("object number mismatch"));
        }
        let end_offset = objstm.entries.get(index + 1)
            .map(|entry| entry.1.try_into().unwrap())
            .unwrap_or(objstm.source.len());
        let mut source = &objstm.source[start_offset.try_into().unwrap()..end_offset];
        ObjParser::read_obj(&mut source)
    }

    fn read_cache_objstm(&self, ostm_num: ObjNum) -> Box<dyn Deref<Target =  Result<ObjStm, Error>> + '_> {
        let ostm_oref = ObjRef { num: ostm_num, gen: 0 };
        let Some(Record::Used { offset: ostm_offset, gen: 0 }) = self.xref.locate(&ostm_oref) else {
            return Box::new(&Err(Error::Parse("object stream not located")));
        };
        if let Entry::Vacant(entry) = self.objstms.borrow_mut().entry(ostm_offset) {
            entry.insert(self.read_objstm(ostm_offset, &ostm_oref));
        }
        Box::new(Ref::map(self.objstms.borrow(), |objstms| objstms.get(&ostm_offset).unwrap()))
    }

    fn read_objstm(&self, ostm_offset: Offset, ostm_oref: &ObjRef) -> Result<ObjStm, Error> {
        let Object::Stream(stm) = self.read_uncompressed(ostm_offset, ostm_oref)? else {
            return Err(Error::Parse("object stream not found"));
        };
        // FIXME: /Type = /ObjStm
        let count = stm.dict.lookup(b"N").num_value()
            .ok_or(Error::Parse("malformed object stream (/N)"))?;
        let first = stm.dict.lookup(b"First").num_value()
            .ok_or(Error::Parse("malformed object stream (/First)"))?;
        let mut reader = self.read_stream_data(&stm)?;
        let mut header = (&mut reader).take(first);
        use crate::parser::Tokenizer;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let num = utils::parse_num::<ObjNum>(&header.read_token()?)
                .ok_or(Error::Parse("malformed object stream header"))?;
            let offset = utils::parse_num::<Offset>(&header.read_token()?)
                .ok_or(Error::Parse("malformed object stream header"))?;
            entries.push((num, offset));
        }
        // Drain the rest of header: https://stackoverflow.com/a/42247224
        std::io::copy(&mut header, &mut std::io::sink())?;
        let mut source = Vec::new();
        std::io::copy(&mut reader, &mut source)?;
        source.shrink_to_fit();
        Ok(ObjStm { entries, source })
    }

    pub fn resolve_deep(&self, obj: &Object) -> Result<Object, Error> {
        Ok(match self.resolve_obj(obj)? {
            Object::Array(arr) =>
                Object::Array(arr.into_iter()
                    .map(|obj| self.resolve_obj(&obj))
                    .collect::<Result<Vec<_>, _>>()?),
            Object::Dict(dict) =>
                Object::Dict(Dict(dict.0.into_iter()
                    .map(|(name, obj)| -> Result<(Name, Object), Error> {
                        Ok((name, self.resolve_obj(&obj)?))
                    })
                    .collect::<Result<Vec<_>, _>>()?)),
            obj => obj
        })
    }

    pub fn read_stream_data(&self, obj: &Stream) -> Result<Box<dyn BufRead + '_>, Error> {
        let Data::Ref(offset) = obj.data else { panic!("read_stream_data called on detached Stream") };
        let len = self.resolve_obj(obj.dict.lookup(b"Length"))?.num_value();
        let filters = self.resolve_deep(obj.dict.lookup(b"Filter"))?;
        let params = match obj.dict.lookup(b"DecodeParms") {
            Object::Dict(dict) => Some(dict),
            &Object::Null => None,
            _ => return Err(Error::Parse("malformed /DecodeParms"))
        };
        let reader = self.parser.read_raw(offset)?;
        let codec_in: Box<dyn BufRead> = match len {
            Some(len) => Box::new(reader.take(len)),
            None => {
                log::warn!("Stream with invalid or missing /Length found, reading until endstream.");
                Box::new(EndstreamReader::new(reader))
            }
        };
        let codec_out = codecs::decode(codec_in, &codecs::to_filters(&filters)?, params);
        Ok(codec_out)
    }
}

impl Locator for Rc<XRefLink> {
    fn locate(&self, objref: &ObjRef) -> Option<Record> {
        self.curr.locate(objref)
            .or_else(|| self.next.as_ref()?.locate(objref))
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

    #[test]
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
    }

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
