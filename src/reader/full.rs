use std::io::{BufRead, Seek};
use std::collections::BTreeMap;
use std::rc::Rc;

use crate::base::*;
use crate::base::types::*;
use crate::parser::FileParser;

use super::base::BaseReader;

pub struct FullReader<T: BufRead + Seek> {
    base: BaseReader<T>,
    xrefs: BTreeMap<Offset, Rc<XRefLink>>,
    entry: Option<Offset>,
}

struct XRefLink {
    curr: XRef,
    next: Option<Rc<XRefLink>>
}

impl<T: BufRead + Seek> FullReader<T> {
    pub fn new(source: T) -> Self {
        let parser = FileParser::new(source);
        let entry = match parser.entrypoint() {
            Ok(offset) => Some(offset),
            Err(err) => {
                log::error!("Entrypoint not found: {err}");
                None
            }
        };
        let base = BaseReader::new(parser);
        let xrefs = BTreeMap::new();
        let mut reader = Self { base, xrefs, entry };
        if let Some(offset) = &reader.entry {
            reader.build_xref_list(*offset);
        }
        reader
    }

    fn build_xref_list(&mut self, entry: Offset) {
        let mut order = Vec::new();
        let mut next_rc = None;
        for (offset, xref) in BaseReader::read_xref_chain(&self.base.parser, entry) {
            if order.iter().any(|(o, _)| o == &offset) {
                log::warn!("Breaking xref chain detected at {offset}.");
                break;
            }
            if let Some(rc) = self.xrefs.get(&offset) {
                next_rc = Some(rc.clone());
                break;
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
                    Some((objref, self.base.read_uncompressed(offset, &objref)
                            .map(|obj| (obj, link))))
                },
                Record::Compr{num_within, index} => {
                    let objref = ObjRef{num, gen: 0};
                    Some((objref, self.base.read_compressed(num_within, index, &link, &objref)
                            .map(|obj| (obj, link))))
                },
                Record::Free{..} => None
            })
    }

    pub fn base_locator(&self) -> &dyn Locator {
        self.entry
            .and_then(|offset| self.xrefs.get(&offset))
            .map(|rc| rc as &dyn Locator)
            .unwrap_or(&() as &dyn Locator)
    }

    pub fn resolve_ref(&self, objref: &ObjRef, locator: &dyn Locator) -> Result<Object, Error> {
        self.base.resolve_ref(objref, locator)
    }

    pub fn resolve_obj(&self, obj: &Object, locator: &dyn Locator) -> Result<Object, Error> {
        self.base.resolve_obj(obj, locator)
    }

    pub fn resolve_deep(&self, obj: &Object, locator: &dyn Locator) -> Result<Object, Error> {
        self.base.resolve_deep(obj, locator)
    }

    pub fn read_stream_data(&self, obj: &Stream, locator: &dyn Locator) -> Result<Box<dyn BufRead + '_>, Error> {
        self.base.read_stream_data(obj, locator)
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
}
