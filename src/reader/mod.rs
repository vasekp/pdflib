use std::io::{BufRead, Seek};
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use crate::base::*;
use crate::base::types::*;
use crate::parser::FileParser;

pub struct Reader<T: BufRead + Seek> {
    parser: FileParser<T>,
    xrefs: BTreeMap<Offset, XRef>,
    entry: Option<Offset>
}

impl<T: BufRead + Seek> Reader<T> {
    pub fn new(source: T) -> Self {
        let mut parser = FileParser::new(source);
        let xrefs = BTreeMap::new();
        let entry = parser.entrypoint();
        if let Err(err) = &entry {
            log::error!("Entrypoint not found: {err}");
        }
        let mut reader = Reader { parser, xrefs, entry: entry.ok() };
        if let Some(offset) = &reader.entry {
            reader.add_xref(*offset);
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
        let xref = match self.parser.read_xref_at(offset) {
            Ok(xref) => xref,
            Err(err) => {
                log::error!("Error reading xref at {offset}: {err}");
                return;
            }
        };
        let xref = entry.insert(xref);
        let prev = xref.dict.lookup(b"Prev").num_value();
        let xrefstm = xref.dict.lookup(b"XRefStm").num_value();
        [xrefstm, prev].into_iter().flatten()
            .for_each(|offset| self.add_xref(offset));
    }

    pub fn objects(&mut self) -> impl Iterator<Item = (ObjRef, Result<(ObjRef, Object), Error>)> + '_ {
        let xrefs = match self.entry {
            Some(entry) => Self::build_xref_list(&self.xrefs, entry),
            _ => vec![]
        };
        let parser = &mut self.parser;
        xrefs.clone().into_iter().enumerate()
            .flat_map(|(index, xref)| xref.map.iter().map(move |(num, rec)| (num, rec, index)))
            .filter(|(_, rec, _)| !matches!(rec, Record::Free{..}))
            // all used objects in all xrefs + back-reference to section
            .map(move |(&num, rec, _index)| match rec {
                &Record::Used{gen, offset} => {
                    let objref = ObjRef{num, gen};
                    (objref, parser.read_obj_at(offset/*, &xrefs[index..]*/))
                },
                _ => todo!("compressed objects")
            })
    }

    fn build_xref_list(xrefs: &BTreeMap<Offset, XRef>, entry: Offset) -> Vec<&XRef> {
        let mut ret: Vec<&XRef> = Vec::new();
        let mut next = Some(entry);
        while let Some(offset) = next.take() {
            let Some(xref) = xrefs.get(&offset) else { break };
            if ret.iter().any(|&other| std::ptr::eq(other, xref)) {
                log::warn!("XRef chain detected, breaking");
                break;
            }
            ret.push(xref);
            'a: {
                let XRefType::Table = xref.tpe else { break 'a };
                let Some(xrefstm) = xref.dict.lookup(b"XRefStm").num_value() else { break 'a };
                let Some(xref) = xrefs.get(&xrefstm) else { break 'a };
                if ret.iter().any(|&other| std::ptr::eq(other, xref)) { break 'a; }
                ret.push(xref);
            }
            next = xref.dict.lookup(b"Prev").num_value();
        }
        ret
    }
}
