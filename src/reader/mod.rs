use std::io::{BufRead, Seek};
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use crate::base::*;
use crate::base::types::*;
use crate::parser::Parser;

pub struct Reader<T: BufRead + Seek> {
    parser: Parser<T>,
    xrefs: BTreeMap<Offset, Result<XRef, Error>>,
    entry: Result<Offset, Error>
}

#[derive(Debug)]
struct XRef {
    tpe: XRefType,
    map: BTreeMap<ObjNum, Record>,
    trailer: Result<Dict, Error>,
    size: Option<ObjNum>,
    prev: [Option<Offset>; 2]
}

impl<T: BufRead + Seek> Reader<T> {
    pub fn new(source: T) -> Self {
        let mut parser = Parser::new(source);
        let xrefs = BTreeMap::new();
        let entry = parser.entrypoint();
        let mut reader = Reader { parser, xrefs, entry };
        if let &Ok(offset) = &reader.entry {
            reader.add_xref(offset);
        }
        for (off, xref) in &reader.xrefs {
            println!("{off}: {xref:?}");
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
                Entry::Occupied(_) =>
                    eprintln!("Duplicate number in xref @ {offset}: {num}") // FIXME store duplicates somewhere
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
        let xref = XRef { tpe, map, trailer, size, prev: [xrefstm, prev] };
        entry.insert(Ok(xref));
        [xrefstm, prev].into_iter().flatten()
            .for_each(|offset| self.add_xref(offset));
    }
}
