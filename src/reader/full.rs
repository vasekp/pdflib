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

pub struct FullReader<T: BufRead + Seek> {
    parser: FileParser<T>,
    xrefs: BTreeMap<Offset, Rc<XRefLink>>,
    entry: Option<Offset>,
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

impl<T: BufRead + Seek> FullReader<T> {
    pub fn new(source: T) -> Self {
        let parser = FileParser::new(source);
        let xrefs = BTreeMap::new();
        let entry = match parser.entrypoint() {
            Ok(offset) => Some(offset),
            Err(err) => {
                log::error!("Entrypoint not found: {err}");
                None
            }
        };
        let mut reader = Self { parser, xrefs, entry, objstms: Default::default() };
        if let Some(offset) = &reader.entry {
            reader.build_xref_list(*offset);
        }
        reader
    }

    fn build_xref_list(&mut self, entry: Offset) {
        let mut queue = VecDeque::from([(entry, false)]);
        let mut order = Vec::new();
        let mut next_rc = None;
        while let Some((offset, is_aside)) = queue.pop_front() {
            if order.iter().any(|(o, _)| o == &offset) {
                log::warn!("Breaking xref chain detected at {offset}.");
                break;
            }
            if let Some(rc) = self.xrefs.get(&offset) {
                next_rc = Some(rc.clone());
                break;
            }
            let xref = match self.parser.read_xref_at(offset) {
                Ok(xref) => xref,
                Err(err) => {
                    log::error!("Error reading xref at {offset}: {err}");
                    break;
                }
            };
            if matches!(xref.tpe, XRefType::Table) {
                if let Some(offset) = xref.dict.lookup(b"XRefStm").num_value() {
                    if !is_aside {
                        queue.push_back((offset, true));
                    } else {
                        log::warn!("/XRefStm pointed to a classical section.");
                    }
                }
            }
            if let Some(offset) = xref.dict.lookup(b"Prev").num_value() {
                if !is_aside {
                    queue.push_back((offset, false));
                } else {
                    log::warn!("Ignoring /Prev in a /XRefStm.");
                }
            }
            order.push((offset, xref));
        }
        for (offset, xref) in order.into_iter().rev() {
            let rc = Rc::new(XRefLink { curr: xref, next: next_rc });
            self.xrefs.insert(offset, Rc::clone(&rc));
            next_rc = Some(rc);
        }
    }

    pub fn objects(&self) -> impl Iterator<Item = (ObjRef, Result<(Object, impl Locator), Error>)> + '_ {
        self.xrefs.iter()
            .flat_map(|(_, rc)| rc.curr.map.iter().map(move |(num, rec)| (num, rec, Rc::clone(rc))))
            // all used objects in all xrefs + back-reference to section
            .flat_map(move |(&num, rec, link)| match *rec {
                Record::Used{gen, offset} => {
                    let objref = ObjRef{num, gen};
                    Some((objref, self.read_uncompressed(offset, &objref)
                            .map(|obj| (obj, link))))
                },
                Record::Compr{num_within, index} => {
                    let objref = ObjRef{num, gen: 0};
                    Some((objref, self.read_compressed(num_within, index, &link, &objref)
                            .map(|obj| (obj, link))))
                },
                Record::Free{..} => None
            })
    }

    pub fn resolve_ref(&self, objref: &ObjRef, locator: &dyn Locator) -> Result<Object, Error> {
        match locator.locate(objref) {
            Some(Record::Used { offset, .. }) => self.read_uncompressed(offset, objref),
            Some(Record::Compr { num_within, index }) => self.read_compressed(num_within, index, locator, objref),
            _ => Ok(Object::Null)
        }
    }

    pub fn resolve_obj(&self, obj: &Object, locator: &dyn Locator) -> Result<Object, Error> {
        match obj {
            Object::Ref(objref) => self.resolve_ref(objref, locator),
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

    pub fn base_locator(&self) -> &dyn Locator {
        self.entry
            .and_then(|offset| self.xrefs.get(&offset))
            .map(|rc| rc as &dyn Locator)
            .unwrap_or(&() as &dyn Locator)
    }

    fn read_compressed(&self, num_within: ObjNum, index: ObjIndex, locator: &dyn Locator, oref_expd: &ObjRef) -> Result<Object, Error> {
        let index = index as usize;
        let cache_ref = self.read_cache_objstm(num_within, locator);
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

    fn read_cache_objstm(&self, ostm_num: ObjNum, locator: &dyn Locator) -> Box<dyn Deref<Target =  Result<ObjStm, Error>> + '_> {
        let ostm_oref = ObjRef { num: ostm_num, gen: 0 };
        let Some(Record::Used { offset: ostm_offset, gen: 0 }) = locator.locate(&ostm_oref) else {
            return Box::new(&Err(Error::Parse("object stream not located")));
        };
        if let Entry::Vacant(entry) = self.objstms.borrow_mut().entry(ostm_offset) {
            entry.insert(self.read_objstm(ostm_offset, &ostm_oref, locator));
        }
        Box::new(Ref::map(self.objstms.borrow(), |objstms| objstms.get(&ostm_offset).unwrap()))
    }

    fn read_objstm(&self, ostm_offset: Offset, ostm_oref: &ObjRef, locator: &dyn Locator) -> Result<ObjStm, Error> {
        let Object::Stream(stm) = self.read_uncompressed(ostm_offset, ostm_oref)? else {
            return Err(Error::Parse("object stream not found"));
        };
        // FIXME: /Type = /ObjStm
        let count = stm.dict.lookup(b"N").num_value()
            .ok_or(Error::Parse("malformed object stream (/N)"))?;
        let first = stm.dict.lookup(b"First").num_value()
            .ok_or(Error::Parse("malformed object stream (/First)"))?;
        let mut reader = self.read_stream_data(&stm, &Uncompressed(locator))?;
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

    pub fn resolve_deep(&self, obj: &Object, locator: &dyn Locator) -> Result<Object, Error> {
        Ok(match self.resolve_obj(obj, locator)? {
            Object::Array(arr) =>
                Object::Array(arr.into_iter()
                    .map(|obj| self.resolve_obj(&obj, locator))
                    .collect::<Result<Vec<_>, _>>()?),
            Object::Dict(dict) =>
                Object::Dict(Dict(dict.0.into_iter()
                    .map(|(name, obj)| -> Result<(Name, Object), Error> {
                        Ok((name, self.resolve_obj(&obj, locator)?))
                    })
                    .collect::<Result<Vec<_>, _>>()?)),
            obj => obj
        })
    }

    pub fn read_stream_data(&self, obj: &Stream, locator: &dyn Locator) -> Result<Box<dyn BufRead + '_>, Error>
    {
        let Data::Ref(offset) = obj.data else { panic!("read_stream_data called on detached Stream") };
        let len = self.resolve_obj(obj.dict.lookup(b"Length"), locator)?.num_value();
        let filters = self.resolve_deep(obj.dict.lookup(b"Filter"), locator)?;
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

struct Uncompressed<'a, T: Locator + ?Sized>(&'a T);

impl<T: Locator + ?Sized> Locator for Uncompressed<'_, T> {
    fn locate(&self, objref: &ObjRef) -> Option<Record> {
        let rec = self.0.locate(objref);
        if matches!(rec, Some(Record::Compr{..})) {
            log::warn!("Object {objref} should be uncompressed.");
            Some(Record::default())
        } else {
            rec
        }
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
        let rdr = FullReader::new(BufReader::new(File::open("src/tests/basic.pdf").unwrap()));
        let mut iter = rdr.objects();

        let (oref, res) = iter.next().unwrap();
        let (obj, link) = res.unwrap();
        assert_eq!(oref, ObjRef { num: 1, gen: 0 });
        assert_eq!(obj, Object::Dict(Dict(vec![
            (Name::from(b"Type"), Object::new_name(b"Pages")),
            (Name::from(b"Kids"), Object::Array(vec![Object::Ref(ObjRef { num: 2, gen: 0 })])),
            (Name::from(b"Count"), Object::Number(Number::Int(1))),
        ])));
        let kids = rdr.resolve_ref(&ObjRef { num: 2, gen: 0 }, &link).unwrap();

        let (oref, res) = iter.next().unwrap();
        let (obj, _) = res.unwrap();
        assert_eq!(oref, ObjRef { num: 2, gen: 0 });
        assert_eq!(obj, kids);

        let mut iter = iter.skip(1);

        let (oref, res) = iter.next().unwrap();
        assert_eq!(oref, ObjRef { num: 4, gen: 0 });
        let (obj, link) = res.unwrap();
        let Object::Stream(stm) = obj else { panic!() };
        let mut data = rdr.read_stream_data(&stm, &link).unwrap();
        let line = data.read_line_excl().unwrap();
        assert_eq!(line, b"1 0 0 -1 0 841.889771 cm");

        //etc.
    }

    #[test]
    fn test_resolve_deep() {
        let rdr = FullReader::new(BufReader::new(File::open("src/tests/indirect-filters.pdf").unwrap()));
        let loc = rdr.base_locator();
        let obj = rdr.resolve_ref(&ObjRef { num: 4, gen: 0 }, loc).unwrap();
        let Object::Stream(Stream { dict, .. }) = obj else { panic!() };
        let fil = dict.lookup(b"Filter");
        let res = rdr.resolve_deep(&fil, loc).unwrap();
        assert_eq!(res, Object::Array(vec![ Object::new_name(b"AsciiHexDecode"), Object::new_name(b"FlateDecode")]));
    }

    #[test]
    fn test_xref_chaining() {
        let rdr = FullReader::new(BufReader::new(File::open("src/tests/hybrid.pdf").unwrap()));
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

        let rdr = FullReader::new(BufReader::new(File::open("src/tests/updates.pdf").unwrap()));
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

        let rdr = FullReader::new(BufReader::new(File::open("src/tests/circular.pdf").unwrap()));
        assert_eq!(rdr.entry, Some(9));
        let x9 = rdr.xrefs.get(&9).unwrap();
        // illegal /Prev (leading to itself)
        assert_eq!(x9.curr.dict.lookup(b"Prev"), &Object::Number(Number::Int(9)));
        // not propagated into the linked list
        assert!(x9.next.is_none());
    }

    #[test]
    fn test_objstm_caching() {
        let rdr = FullReader::new(BufReader::new(File::open("src/tests/objstm.pdf").unwrap()));
        let loc = rdr.base_locator();
        assert_eq!(loc.locate(&ObjRef { num: 1, gen: 0 }), Some(Record::Compr { num_within: 8, index: 4 }));
        assert!(rdr.objstms.borrow().is_empty());
        let obj = rdr.resolve_ref(&ObjRef { num: 1, gen: 0 }, loc).unwrap();
        assert_eq!(obj, Object::Dict(Dict(vec![
            (Name::from(b"Pages"), Object::Ref(ObjRef { num: 9, gen: 0 })),
            (Name::from(b"Type"), Object::new_name(b"Catalog")),
        ])));

        let objstms = rdr.objstms.borrow();
        assert!(!objstms.is_empty());
        let line = objstms.get(&4973).unwrap().as_ref().unwrap().source.as_slice().read_line_excl().unwrap();
        assert_eq!(line, b"<</Font<</F1 5 0 R>>/ProcSet[/PDF/Text/ImageC/ImageB/ImageI]>>");
        drop(objstms);

        let obj2 = rdr.resolve_ref(&ObjRef { num: 1, gen: 0 }, loc).unwrap();
        assert_eq!(obj, obj2);
    }

    #[test]
    fn test_read_objstm_take() {
        let source = "1 0 obj <</Type/ObjStm /N 3 /First 11 /Length 14>> stream
2 0 3 1 4 2614
endstream endobj";
        let rdr = FullReader::new(Cursor::new(source));
        let objstm = rdr.read_objstm(0, &ObjRef { num: 1, gen: 0 }, &()).unwrap();
        assert_eq!(objstm.entries, vec![(2, 0), (3, 1), (4, 2)]);
        assert_eq!(objstm.source, b"614");

        struct MockLocator();
        impl Locator for MockLocator {
            fn locate(&self, objref: &ObjRef) -> Option<Record> {
                match objref.num {
                    1 => Some(Record::Used { gen: 0, offset: 0 }),
                    2..=4 => Some(Record::Compr { num_within: 1, index: (objref.num as ObjIndex) - 2 }),
                    _ => panic!()
                }
            }
        }
        let loc = MockLocator();
        assert_eq!(rdr.resolve_ref(&ObjRef { num: 2, gen: 0 }, &loc).unwrap(),
            Object::Number(Number::Int(6)));
        assert_eq!(rdr.resolve_ref(&ObjRef { num: 3, gen: 0 }, &loc).unwrap(),
            Object::Number(Number::Int(1)));
        assert_eq!(rdr.resolve_ref(&ObjRef { num: 4, gen: 0 }, &loc).unwrap(),
            Object::Number(Number::Int(4)));
    }

    #[test]
    fn test_read_stream_overflow() {
        let source = "1 0 obj <</Length 10>> stream\n123\nendstream endobj";
        let rdr = FullReader::new(Cursor::new(source));
        let Object::Stream(stm) = rdr.read_uncompressed(0, &ObjRef { num: 1, gen: 0 }).unwrap()
            else { panic!() };
        let mut data = rdr.read_stream_data(&stm, &()).unwrap();
        let mut s = String::new();
        data.read_to_string(&mut s).unwrap();
        drop(data);
        assert_eq!(s, "123\nendstr");

        let source = "1 0 obj <</Length 100>> stream\n123\nendstream endobj";
        let rdr = FullReader::new(Cursor::new(source));
        let Object::Stream(stm) = rdr.read_uncompressed(0, &ObjRef { num: 1, gen: 0 }).unwrap()
            else { panic!() };
        let mut data = rdr.read_stream_data(&stm, &()).unwrap();
        let mut s = String::new();
        data.read_to_string(&mut s).unwrap();
        drop(data);
        assert_eq!(s, "123\nendstream endobj");

        let source = "1 0 obj <</Length 10>> stream\n123";
        let rdr = FullReader::new(Cursor::new(source));
        let Object::Stream(stm) = rdr.read_uncompressed(0, &ObjRef { num: 1, gen: 0 }).unwrap()
            else { panic!() };
        let mut data = rdr.read_stream_data(&stm, &()).unwrap();
        let mut s = String::new();
        data.read_to_string(&mut s).unwrap();
        drop(data);
        assert_eq!(s, "123");

        let source = "1 0 obj <<>> stream\n123\n45endstream endobj";
        let rdr = FullReader::new(Cursor::new(source));
        let Object::Stream(stm) = rdr.read_uncompressed(0, &ObjRef { num: 1, gen: 0 }).unwrap()
            else { panic!() };
        let mut data = rdr.read_stream_data(&stm, &()).unwrap();
        let mut s = String::new();
        data.read_to_string(&mut s).unwrap();
        drop(data);
        assert_eq!(s, "123\n45");

        let source = "1 0 obj <<>> stream\n123";
        let rdr = FullReader::new(Cursor::new(source));
        let Object::Stream(stm) = rdr.read_uncompressed(0, &ObjRef { num: 1, gen: 0 }).unwrap()
            else { panic!() };
        let mut data = rdr.read_stream_data(&stm, &()).unwrap();
        let mut s = String::new();
        data.read_to_string(&mut s).unwrap();
        drop(data);
        assert_eq!(s, "123");
    }
}
