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

pub struct Reader<T: BufRead + Seek> {
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

impl<T: BufRead + Seek> Reader<T> {
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
        let mut reader = Reader { parser, xrefs, entry, objstms: Default::default() };
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

    pub fn resolve(&self, obj: &Object, locator: &dyn Locator) -> Result<Object, Error> {
        let Object::Ref(objref) = obj else {
            return Ok(obj.to_owned());
        };
        match locator.locate(objref) {
            Some(Record::Used { offset, .. }) => self.read_uncompressed(offset, objref),
            Some(Record::Compr { num_within, index }) => self.read_compressed(num_within, index, locator, objref),
            _ => Ok(Object::Null)
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
            entry.insert(self.read_objstm(ostm_offset, ostm_oref, locator));
        }
        Box::new(Ref::map(self.objstms.borrow(), |objstms| objstms.get(&ostm_offset).unwrap()))
    }

    fn read_objstm(&self, ostm_offset: Offset, ostm_oref: ObjRef, locator: &dyn Locator) -> Result<ObjStm, Error> {
        let Object::Stream(stm) = self.read_uncompressed(ostm_offset, &ostm_oref)? else {
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
        Ok(match self.resolve(obj, locator)? {
            Object::Array(arr) =>
                Object::Array(arr.into_iter()
                    .map(|obj| self.resolve(&obj, locator))
                    .collect::<Result<Vec<_>, _>>()?),
            Object::Dict(dict) =>
                Object::Dict(Dict(dict.0.into_iter()
                    .map(|(name, obj)| -> Result<(Name, Object), Error> {
                        Ok((name, self.resolve(&obj, locator)?))
                    })
                    .collect::<Result<Vec<_>, _>>()?)),
            obj => obj
        })
    }

    pub fn read_stream_data(&self, obj: &Stream, locator: &dyn Locator) -> Result<Box<dyn BufRead + '_>, Error>
    {
        let Data::Ref(offset) = obj.data else { panic!("read_stream_data called on detached Stream") };
        let len = self.resolve(obj.dict.lookup(b"Length"), locator)?.num_value().unwrap(); // TODO
        let filters = self.resolve_deep(obj.dict.lookup(b"Filter"), locator)?;
        let params = match obj.dict.lookup(b"DecodeParms") {
            Object::Dict(dict) => Some(dict),
            &Object::Null => None,
            _ => return Err(Error::Parse("malformed /DecodeParms"))
        };
        let reader = self.parser.read_raw(offset)?;
        let codec_in = reader.take(len);
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
