use std::io::{Read, BufRead, Seek};
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;

use crate::base::*;
use crate::base::types::*;
use crate::parser::{FileParser, ObjParser};
use crate::codecs;
use crate::utils;

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
            // all used objects in all xrefs + back-reference to section
            .flat_map(move |(&num, rec, link)| match *rec {
                Record::Used{gen, offset} => {
                    let objref = ObjRef{num, gen};
                    let res = match parser.read_obj_at(offset) {
                        Err(err) => Err(err),
                        Ok((rref, _)) if rref != objref => Err(Error::Parse("object number mismatch")),
                        Ok((_, obj)) => Ok((obj, link))
                    };
                    Some((objref, res))
                },
                Record::Compr{num: within, index} => {
                    let objref = ObjRef{num, gen: 0};
                    let res = match self.read_compressed(within, index, &link) {
                        Err(err) => Err(err),
                        Ok((rref, _)) if rref != objref => Err(Error::Parse("object number mismatch")),
                        Ok((_, obj)) => Ok((obj, link))
                    };
                    Some((objref, res))
                },
                Record::Free{..} => None
            })
    }

    pub fn resolve(&self, obj: &Object, locator: &impl Locator) -> Result<Object, Error> {
        let Object::Ref(objref) = obj else {
            return Ok(obj.to_owned());
        };
        match locator.locate(objref) {
            Some(Record::Used { offset, .. }) => {
                let (readref, obj) = self.parser.read_obj_at(offset)?;
                if &readref == objref {
                    Ok(obj)
                } else {
                    Err(Error::Parse("object number mismatch"))
                }
            },
            Some(Record::Compr { num, index }) => {
                let (readref, obj) = self.read_compressed(num, index, locator)?;
                if &readref == objref {
                    Ok(obj)
                } else {
                    Err(Error::Parse("object number mismatch"))
                }
            },
            _ => Ok(Object::Null)
        }
    }

    fn read_compressed(&self, num_within: ObjNum, index: ObjGen, locator: &impl Locator) -> Result<(ObjRef, Object), Error> {
        let oref_ostm = ObjRef { num: num_within, gen: 0 };
        let Some(Record::Used { offset, gen: 0 }) = locator.locate(&oref_ostm) else {
            return Err(Error::Parse("object stream not located"));
        };
        let (rref, obj) = self.parser.read_obj_at(offset)?;
        if rref != oref_ostm {
            return Err(Error::Parse("object number mismatch"));
        };
        let Object::Stream(stm) = obj else {
            return Err(Error::Parse("object stream not found"));
        };
        // FIXME: /Type = /ObjStm
        let count = stm.dict.lookup(b"N").num_value()
            .ok_or(Error::Parse("malformed object stream (/N)"))?;
        if index >= count {
            return Ok((ObjRef { num: 0, gen: 0 }, Object::Null)); // FIXME: placeholder object number!
        }
        let first = stm.dict.lookup(b"First").num_value()
            .ok_or(Error::Parse("malformed object stream (/First)"))?;
        let mut reader = self.read_stream_data(&stm, locator)?;
        let mut header = (&mut reader).take(first);
        use crate::parser::Tokenizer;
        for _ in 0..index {
            header.read_token()?;
            header.read_token()?;
        }
        let rnum = utils::parse_num::<ObjNum>(&header.read_token()?)
            .ok_or(Error::Parse("malformed object stream header"))?;
        let offset = utils::parse_num::<Offset>(&header.read_token()?)
            .ok_or(Error::Parse("malformed object stream header"))?;
        let _ = header.read_token();
        let next_offset = header.read_token().ok()
            .map(|tk| utils::parse_num::<Offset>(&tk).ok_or(Error::Parse("malformed object stream header")))
            .transpose()?;
        // Drain rest of header to get to start of objects.
        loop {
            let avail = match header.fill_buf() {
                Ok(n) => n.len(),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into())
            };
            if avail == 0 {
                break;
            }
            header.consume(avail);
        }
        // Skip first `offset` bytes.
        let mut rem = offset.try_into().expect("Should fit into u64.");
        loop {
            let avail = match reader.fill_buf() {
                Ok(n) => n.len(),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into())
            };
            if avail == 0 {
                break;
            }
            if avail < rem {
                reader.consume(avail);
                rem -= avail;
            } else {
                reader.consume(rem);
                break;
            }
        }
        let obj = if let Some(next_offset) = next_offset {
            ObjParser::read_obj(&mut reader.take(next_offset - offset))
        } else {
            ObjParser::read_obj(&mut reader)
        }?;
        Ok((ObjRef { num: rnum, gen: 0 }, obj))
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

    pub fn read_stream_data<L>(&self, obj: &Stream, locator: &L) -> Result<Box<dyn BufRead + '_>, Error>
        where L: Locator
    {
        let Data::Ref(offset) = obj.data else { panic!("read_stream_data called on detached Stream") };
        let len = self.resolve(obj.dict.lookup(b"Length"), locator)?.num_value().unwrap(); // TODO
        let filters = self.resolve_deep(obj.dict.lookup(b"Filter"), locator)?;
        let reader = self.parser.read_raw(offset)?;
        let codec_in = reader.take(len);
        let codec_out = codecs::decode(codec_in, &codecs::to_filters(&filters)?);
        Ok(codec_out)
    }
}

impl Locator for Rc<XRefLink> {
    fn locate(&self, objref: &ObjRef) -> Option<Record> {
        self.curr.locate(objref)
            .or_else(|| self.next.as_ref()?.locate(objref))
    }
}
