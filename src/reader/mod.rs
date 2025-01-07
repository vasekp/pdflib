use std::io::{BufRead, Seek};
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use crate::base::*;
use crate::parser::Parser;

pub struct Reader<T: BufRead + Seek> {
    parser: Parser<T>,
    xrefs: BTreeMap<u64, Result<XRef, Error>>,
    entry: Result<u64, Error>
}

#[derive(Debug)]
struct XRef {
    tpe: XRefType,
    map: BTreeMap<u64, Record>,
    trailer: Result<Dict, Error>,
    size: Option<u64>,
    prev: [Option<u64>; 2]
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

    fn add_xref(&mut self, offset: u64) {
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
            if map.contains_key(&num) {
                eprintln!("Duplicate number in xref @ {offset}: {num}"); // FIXME store duplicates somewhere
            } else {
                map.insert(num, rec);
            }
        }
        let trailer = iter.trailer();
        let (size, prev, xrefstm) = match &trailer {
            Ok(dict) => (
                match dict.lookup(b"Size") {
                    &Object::Number(Number::Int(size)) if size > 0 => Some(size as u64),
                    _ => None
                },
                match dict.lookup(b"Prev") {
                    &Object::Number(Number::Int(offset)) if offset > 0 => Some(offset as u64),
                    _ => None
                },
                match (&tpe, dict.lookup(b"XRefStm")) {
                    (&XRefType::Table, &Object::Number(Number::Int(offset))) if offset > 0 => Some(offset as u64),
                    _ => None
                }
            ),
            Err(_) => (None, None, None)
        };
        let xref = XRef { tpe, map, trailer, size, prev: [xrefstm, prev] };
        let xref = entry.insert(Ok(xref)).as_ref().unwrap();
        xref.prev.into_iter().flatten().for_each(|offset| self.add_xref(offset));
    }
}
