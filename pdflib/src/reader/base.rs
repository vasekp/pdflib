use std::io::{Read, BufRead, Seek};
use std::collections::{BTreeMap, VecDeque};
use std::collections::btree_map::Entry;
use std::cell::{RefCell, Ref};
use std::ops::Deref;

use crate::base::*;
use crate::base::types::*;
use crate::parser::{FileParser, ObjParser};
use crate::codecs::Filter;
use crate::codecs;
use crate::utils;

use super::esr::EndstreamReader;

pub struct BaseReader<T: BufRead + Seek> {
    pub parser: FileParser<T>,
    objstms: RefCell<BTreeMap<Offset, Result<ObjStm, Error>>>,
}

struct ObjStm {
    entries: Vec<(ObjNum, Offset)>,
    source: Vec<u8>,
}

impl<T: BufRead + Seek> BaseReader<T> {
    pub fn new(parser: FileParser<T>) -> Self {
        Self { parser, objstms: Default::default() }
    }

    pub fn read_xref_chain(parser: &FileParser<T>, entry: Offset) -> impl Iterator<Item = (Offset, XRef)> + use<'_, T> {
        XRefIterator::new(parser, entry)
    }

    pub fn resolve_ref(&self, objref: &ObjRef, locator: &dyn Locator) -> Result<Object, Error> {
        match locator.locate(objref) {
            Some(Record::Used { offset, .. }) => self.read_uncompressed(offset, objref),
            Some(Record::Compr { num_within, index }) => self.read_compressed(num_within, index, locator, objref),
            _ => Ok(Object::Null)
        }
    }

    pub fn resolve_obj(&self, obj: Object, locator: &dyn Locator) -> Result<Object, Error> {
        match obj {
            Object::Ref(objref) => self.resolve_ref(&objref, locator),
            _ => Ok(obj)
        }
    }

    pub fn read_uncompressed(&self, offset: Offset, oref_expd: &ObjRef) -> Result<Object, Error> {
        let (oref, obj) = self.parser.read_obj_at(offset)?;
        if &oref == oref_expd {
            Ok(obj)
        } else {
            Err(Error::Parse("object number mismatch"))
        }
    }

    pub fn read_compressed(&self, num_within: ObjNum, index: ObjIndex, locator: &dyn Locator, oref_expd: &ObjRef) -> Result<Object, Error> {
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
        let stm = self.read_uncompressed(ostm_offset, ostm_oref)?
            .into_stream()
            .ok_or(Error::Parse("object stream not found"))?;
        // FIXME: /Type = /ObjStm
        let count = stm.dict.lookup(b"N").num_value()
            .ok_or(Error::Parse("malformed object stream (/N)"))?;
        let first = stm.dict.lookup(b"First").num_value()
            .ok_or(Error::Parse("malformed object stream (/First)"))?;
        let mut reader = self.read_stream_data(&stm, locator)?;
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

    pub fn resolve_deep(&self, obj: Object, locator: &dyn Locator) -> Result<Object, Error> {
        Ok(match self.resolve_obj(obj, locator)? {
            Object::Array(arr) =>
                Object::Array(arr.into_iter()
                    .map(|obj| self.resolve_obj(obj, locator))
                    .collect::<Result<Vec<_>, _>>()?),
            Object::Dict(dict) =>
                Object::Dict(Dict::from(dict.into_iter()
                    .map(|(name, obj)| -> Result<(Name, Object), Error> {
                        Ok((name, self.resolve_obj(obj, locator)?))
                    })
                    .collect::<Result<Vec<_>, _>>()?)),
            obj => obj
        })
    }

    pub fn resolve_filters(&self, obj: &Object, locator: &dyn Locator) -> Result<Vec<Filter>, Error> {
        let binding;
        let obj_res = match obj {
            Object::Ref(objref) => {
                binding = self.resolve_ref(objref, locator)?;
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
                            binding = self.resolve_ref(objref, locator)?;
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

    pub fn read_stream_data(&self, obj: &Stream, locator: &dyn Locator) -> Result<Box<dyn BufRead + '_>, Error>
    {
        let Data::Ref(offset) = obj.data else { panic!("read_stream_data called on detached Stream") };
        let len = self.resolve_obj(obj.dict.lookup(b"Length").to_owned(), locator)?.num_value();
        let filters = self.resolve_filters(obj.dict.lookup(b"Filter"), locator)?;
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
        let codec_out = codecs::decode(codec_in, &filters, params);
        Ok(codec_out)
    }
}

struct XRefIterator<'a, T: BufRead + Seek> {
    parser: &'a FileParser<T>,
    queue: VecDeque<(Offset, bool)>,
}

impl<'a, T: BufRead + Seek> XRefIterator<'a, T> {
    fn new(parser: &'a FileParser<T>, entry: Offset) -> Self {
        Self { parser, queue: VecDeque::from([(entry, false)]) }
    }
}

impl<T: BufRead + Seek> Iterator for XRefIterator<'_, T> {
    type Item = (Offset, XRef);

    fn next(&mut self) -> Option<Self::Item> {
        let (offset, is_aside) = self.queue.pop_front()?;
        let xref = match self.parser.read_xref_at(offset) {
            Ok(xref) => xref,
            Err(err) => {
                log::error!("Error reading xref at {offset}: {err}");
                return None;
            }
        };
        if matches!(xref.tpe, XRefType::Table) {
            if let Some(offset) = xref.dict.lookup(b"XRefStm").num_value() {
                if !is_aside {
                    self.queue.push_back((offset, true));
                } else {
                    log::warn!("/XRefStm pointed to a classical section.");
                }
            }
        }
        if let Some(offset) = xref.dict.lookup(b"Prev").num_value() {
            if !is_aside {
                self.queue.push_back((offset, false));
            } else {
                log::warn!("Ignoring /Prev in a /XRefStm.");
            }
        }
        Some((offset, xref))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::*;
    use std::fs::*;

    #[test]
    fn test_resolve_filters() {
        let fp = FileParser::new(BufReader::new(File::open("src/tests/indirect-filters.pdf").unwrap()));
        let xref = fp.read_xref_at(fp.entrypoint().unwrap()).unwrap();
        let rdr = BaseReader::new(fp);
        let Stream { dict, ..} = rdr.resolve_ref(&ObjRef { num: 4, gen: 0 }, &xref)
            .unwrap()
            .into_stream()
            .unwrap();
        let fil = dict.lookup(b"Filter");
        let res = rdr.resolve_filters(fil, &xref).unwrap();
        assert_eq!(res, vec![ Filter::AsciiHex, Filter::Flate]);
    }

    #[test]
    fn test_xref_chaining() {
        let fp = FileParser::new(BufReader::new(File::open("src/tests/hybrid.pdf").unwrap()));
        let mut iter = BaseReader::read_xref_chain(&fp, fp.entrypoint().unwrap());
        assert_eq!(iter.next().unwrap().0, 912);
        // main table's /XRefStm
        assert_eq!(iter.next().unwrap().0, 759);
        // main table's /Prev
        assert_eq!(iter.next().unwrap().0, 417);
        assert!(iter.next().is_none());

        let fp = FileParser::new(BufReader::new(File::open("src/tests/updates.pdf").unwrap()));
        let mut iter = BaseReader::read_xref_chain(&fp, fp.entrypoint().unwrap());
        assert_eq!(iter.next().unwrap().0, 510);
        // main table's /Prev
        assert_eq!(iter.next().unwrap().0, 322);
        // /Prev's /Prev
        assert_eq!(iter.next().unwrap().0, 87);
        assert!(iter.next().is_none());
        drop(iter);

        let fp = FileParser::new(BufReader::new(File::open("src/tests/circular.pdf").unwrap()));
        let mut iter = BaseReader::read_xref_chain(&fp, fp.entrypoint().unwrap());
        assert_eq!(iter.next().unwrap().0, 9);
        assert_eq!(iter.next().unwrap().0, 9);
    }

    #[test]
    fn test_read_stream_data() {
        // Direct length
        let fp = FileParser::new(BufReader::new(File::open("src/tests/hybrid.pdf").unwrap()));
        let rdr = BaseReader::new(fp);
        let stm = rdr.read_uncompressed(251, &ObjRef { num: 4, gen: 0 })
            .unwrap()
            .into_stream()
            .unwrap();
        let mut data = rdr.read_stream_data(&stm, &()).unwrap();
        let mut s = Vec::new();
        data.read_to_end(&mut s).unwrap();
        drop(data);
        assert_eq!(s, b"BT /F1 12 Tf 72 720 Td (Hello, PDF 1.5!) Tj ET");

        // Indirect length - does not exclude the final EOL
        let fp = FileParser::new(BufReader::new(File::open("src/tests/updates.pdf").unwrap()));
        let rdr = BaseReader::new(fp);
        let stm = rdr.read_uncompressed(9, &ObjRef { num: 1, gen: 0 })
            .unwrap()
            .into_stream()
            .unwrap();
        let mut data = rdr.read_stream_data(&stm, &()).unwrap();
        let mut s = Vec::new();
        data.read_to_end(&mut s).unwrap();
        drop(data);
        assert_eq!(s, b"Test 1\n");
    }

    #[test]
    fn test_objstm_caching() {
        use crate::parser::bp::ByteProvider;

        let fp = FileParser::new(BufReader::new(File::open("src/tests/objstm.pdf").unwrap()));
        let xref = fp.read_xref_at(fp.entrypoint().unwrap()).unwrap();
        let rdr = BaseReader::new(fp);
        assert_eq!(xref.locate(&ObjRef { num: 1, gen: 0 }), Some(Record::Compr { num_within: 8, index: 4 }));
        assert!(rdr.objstms.borrow().is_empty());
        let obj = rdr.resolve_ref(&ObjRef { num: 1, gen: 0 }, &xref).unwrap();
        assert_eq!(obj, Object::Dict(Dict::from(vec![
            (Name::from(b"Pages"), Object::Ref(ObjRef { num: 9, gen: 0 })),
            (Name::from(b"Type"), Object::new_name(b"Catalog")),
        ])));

        let objstms = rdr.objstms.borrow();
        assert!(!objstms.is_empty());
        let line = objstms.get(&4973).unwrap().as_ref().unwrap().source.as_slice().read_line_excl().unwrap();
        assert_eq!(line, b"<</Font<</F1 5 0 R>>/ProcSet[/PDF/Text/ImageC/ImageB/ImageI]>>");
        drop(objstms);

        let obj2 = rdr.resolve_ref(&ObjRef { num: 1, gen: 0 }, &xref).unwrap();
        assert_eq!(obj, obj2);
    }

    #[test]
    fn test_read_objstm_take() {
        let source = "1 0 obj <</Type/ObjStm /N 3 /First 11 /Length 14>> stream
2 0 3 1 4 2614endstream endobj";
        let rdr = BaseReader::new(FileParser::new(Cursor::new(source)));
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
        let rdr = BaseReader::new(FileParser::new(Cursor::new(source)));
        let stm = rdr.read_uncompressed(0, &ObjRef { num: 1, gen: 0 })
            .unwrap()
            .into_stream()
            .unwrap();
        let mut data = rdr.read_stream_data(&stm, &()).unwrap();
        let mut s = Vec::new();
        data.read_to_end(&mut s).unwrap();
        drop(data);
        assert_eq!(s, b"123\nendstr");

        let source = "1 0 obj <</Length 100>> stream\n123\nendstream endobj";
        let rdr = BaseReader::new(FileParser::new(Cursor::new(source)));
        let stm = rdr.read_uncompressed(0, &ObjRef { num: 1, gen: 0 })
            .unwrap()
            .into_stream()
            .unwrap();
        let mut data = rdr.read_stream_data(&stm, &()).unwrap();
        let mut s = Vec::new();
        data.read_to_end(&mut s).unwrap();
        drop(data);
        assert_eq!(s, b"123\nendstream endobj");

        let source = "1 0 obj <</Length 10>> stream\n123";
        let rdr = BaseReader::new(FileParser::new(Cursor::new(source)));
        let stm = rdr.read_uncompressed(0, &ObjRef { num: 1, gen: 0 })
            .unwrap()
            .into_stream()
            .unwrap();
        let mut data = rdr.read_stream_data(&stm, &()).unwrap();
        let mut s = Vec::new();
        data.read_to_end(&mut s).unwrap();
        drop(data);
        assert_eq!(s, b"123");

        let source = "1 0 obj <<>> stream\n123\n45endstream endobj";
        let rdr = BaseReader::new(FileParser::new(Cursor::new(source)));
        let stm = rdr.read_uncompressed(0, &ObjRef { num: 1, gen: 0 })
            .unwrap()
            .into_stream()
            .unwrap();
        let mut data = rdr.read_stream_data(&stm, &()).unwrap();
        let mut s = Vec::new();
        data.read_to_end(&mut s).unwrap();
        drop(data);
        assert_eq!(s, b"123\n45");

        let source = "1 0 obj <<>> stream\n123";
        let rdr = BaseReader::new(FileParser::new(Cursor::new(source)));
        let stm = rdr.read_uncompressed(0, &ObjRef { num: 1, gen: 0 })
            .unwrap()
            .into_stream()
            .unwrap();
        let mut data = rdr.read_stream_data(&stm, &()).unwrap();
        let mut s = Vec::new();
        data.read_to_end(&mut s).unwrap();
        drop(data);
        assert_eq!(s, b"123");
    }
}
