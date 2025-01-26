use std::io::{BufRead, Seek};
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use crate::base::*;
use crate::base::types::*;
use crate::parser::FileParser;

pub struct Reader<T: BufRead + Seek> {
    parser: FileParser<T>,
    xrefs: BTreeMap<Offset, Result<XRef, Error>>,
    entry: Result<Offset, Error>
}

#[derive(Debug)]
struct XRef {
    tpe: XRefType,
    map: BTreeMap<ObjNum, Record>,
    trailer: Result<Dict, Error>,
    size: Option<ObjNum>
}

impl Locator for XRef {
    fn locate(&self, objref: &ObjRef) -> Option<Record> {
        if self.size.map(|size| objref.num >= size) == Some(true) {
            return Some(Record::default());
        }
        match self.map.get(&objref.num)? {
            rec @ &Record::Used{gen, ..} if gen == objref.gen => Some(*rec),
            rec @ &Record::Compr{..} if objref.gen == 0 => Some(*rec),
            rec @ &Record::Free{..} => Some(*rec),
            _ => Some(Record::default())
        }
    }
}

impl Locator for [&XRef] {
    fn locate(&self, objref: &ObjRef) -> Option<Record> {
        self.iter()
            .flat_map(|xref| xref.locate(objref))
            .next()
    }
}

impl<T: BufRead + Seek> Reader<T> {
    pub fn new(source: T) -> Self {
        let mut parser = FileParser::new(source);
        let xrefs = BTreeMap::new();
        let entry = parser.entrypoint();
        let mut reader = Reader { parser, xrefs, entry };
        if let &Ok(offset) = &reader.entry {
            reader.add_xref(offset);
        }
        for (off, xref) in &reader.xrefs {
            println!("{off}: {xref:?}\n");
        }
        reader
    }

    fn add_xref(&mut self, offset: Offset) {
        let entry = match self.xrefs.entry(offset) {
            Entry::Vacant(entry) => entry,
            Entry::Occupied(_) => return
        };
        let (tpe, mut iter) = match self.parser.read_xref_at(offset) {
            Ok(vals) => vals,
            Err(err) => {
                entry.insert(Err(err));
                return;
            }
        };
        let mut map = BTreeMap::new();
        while let Some(Ok((num, rec))) = iter.next() {
            match map.entry(num) {
                Entry::Vacant(entry) => { entry.insert(rec); },
                Entry::Occupied(_) => log::warn!("Duplicate number in xref @ {offset}: {num}")
            };
        }
        let trailer = iter.trailer();
        let (size, prev, xrefstm) = match &trailer {
            Ok(dict) => (
                dict.lookup(b"Size").num_value(),
                dict.lookup(b"Prev").num_value(),
                dict.lookup(b"XRefStm").num_value()
            ),
            Err(_) => (None, None, None)
        };
        let xref = XRef { tpe, map, trailer, size };
        entry.insert(Ok(xref));
        [xrefstm, prev].into_iter().flatten()
            .for_each(|offset| self.add_xref(offset));
    }

    pub fn objects(&mut self) -> impl Iterator<Item = (ObjRef, Result<(ObjRef, Object), Error>)> + '_ {
        let xrefs = match self.entry {
            Ok(entry) => Self::build_xref_list(&self.xrefs, entry),
            _ => vec![]
        };
        let parser = &mut self.parser;
        xrefs.clone().into_iter().enumerate()
            .flat_map(|(index, xref)| xref.map.iter().map(move |(num, rec)| (num, rec, index)))
            .filter(|(_, rec, _)| !matches!(rec, Record::Free{..}))
            // all used objects in all xrefs + back-reference to section
            .map(move |(&num, rec, index)| match rec {
                &Record::Used{gen, offset} => {
                    let objref = ObjRef{num, gen};
                    (objref, parser.read_obj_at(offset, &xrefs[index..]))
                },
                _ => todo!("compressed objects")
            })
    }

    fn build_xref_list(xrefs: &BTreeMap<Offset, Result<XRef, Error>>, entry: Offset) -> Vec<&XRef> {
        let mut ret: Vec<&XRef> = Vec::new();
        let mut next = Some(entry);
        while let Some(offset) = next.take() {
            let Some(Ok(xref)) = xrefs.get(&offset) else { break };
            if ret.iter().any(|&other| std::ptr::eq(other, xref)) {
                log::warn!("XRef chain detected, breaking");
                break;
            }
            ret.push(xref);
            let Ok(dict) = &xref.trailer else { break };
            'a: {
                let XRefType::Table = xref.tpe else { break 'a };
                let Some(xrefstm) = dict.lookup(b"XRefStm").num_value() else { break 'a };
                let Some(Ok(xref)) = xrefs.get(&xrefstm) else { break 'a };
                if ret.iter().any(|&other| std::ptr::eq(other, xref)) { break 'a; }
                ret.push(xref);
            }
            next = dict.lookup(b"Prev").num_value();
        }
        ret
    }
}
