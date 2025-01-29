use std::io::{BufRead, Seek};
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;

use crate::base::*;
use crate::base::types::*;
use crate::parser::FileParser;

pub struct Reader<T: BufRead + Seek> {
    parser: FileParser<T>,
    xrefs: BTreeMap<Offset, Rc<XRefLink>>,
    entry: Option<Offset>
}

struct XRefLink {
    curr: XRef,
    next: Option<Rc<XRefLink>>
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
        let mut reader = Reader { parser, xrefs, entry };
        if let Some(offset) = &reader.entry {
            reader.build_xref_list(*offset);
        }
        for (offset, link) in &reader.xrefs {
            println!("{offset}: {:?}\n", link.curr);
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
        let parser = &self.parser;
        self.xrefs.iter()
            .flat_map(|(_, rc)| rc.curr.map.iter().map(move |(num, rec)| (num, rec, Rc::clone(rc))))
            .filter(|(_, rec, _)| !matches!(rec, Record::Free{..}))
            // all used objects in all xrefs + back-reference to section
            .map(move |(&num, rec, link)| match rec {
                &Record::Used{gen, offset} => {
                    let objref = ObjRef{num, gen};
                    let res = match parser.read_obj_at(offset) {
                        Err(err) => Err(err),
                        Ok((rref, _)) if rref != objref => Err(Error::Parse("object number mismatch")),
                        Ok((_, obj)) => Ok((obj, link))
                    };
                    (objref, res)
                },
                _ => todo!("compressed objects")
            })
    }

    pub fn resolve(&self, obj: &Object, locator: &impl Locator) -> Result<Object, Error> {
        if let Object::Ref(objref) = obj {
            let Some(offset) = locator.locate_offset(objref) else {
                return Ok(Object::Null)
            };
            let (readref, obj) = self.parser.read_obj_at(offset)?;
            if &readref == objref {
                Ok(obj)
            } else {
                Err(Error::Parse("object number mismatch"))
            }
        } else {
            Ok(obj.to_owned())
        }
    }

    pub fn resolve_deep(&self, obj: &Object, locator: &impl Locator) -> Result<Object, Error> {
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

    pub fn read_stream_data<L>(&self, obj: &Stream, locator: &L) -> Result<impl std::io::BufRead + use<'_, T, L>, Error>
        where L: Locator
    {
        let Data::Ref(offset) = obj.data else { panic!("read_stream_data called on detached Stream") };
        let len = self.resolve(obj.dict.lookup(b"Length"), locator)?.num_value().unwrap(); // TODO
        let filters = match self.resolve_deep(obj.dict.lookup(b"Filter"), locator)? { // TODO separate
            Object::Name(name) => vec![name.to_owned()],
            Object::Array(vec) => vec.iter()
                .map(|obj| match obj {
                    Object::Name(name) => Ok(name.to_owned()),
                    _ => Err(Error::Parse("malformed /Filter"))
                })
                .collect::<Result<Vec<_>, _>>()?,
            Object::Null => vec![],
            _ => return Err(Error::Parse("malformed /Filter"))
        };
        let reader = self.parser.read_raw(offset)?;
        let codec_in = std::io::Read::take(reader, len);
        let codec_out = crate::codecs::decode(codec_in, &filters);
        Ok(codec_out)
    }
}

impl Locator for Rc<XRefLink> {
    fn locate(&self, objref: &ObjRef) -> Option<Record> {
        self.curr.locate(objref)
            .or_else(|| self.next.as_ref()?.locate(objref))
    }
}
