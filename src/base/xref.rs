use std::collections::BTreeMap;

use super::*;
use super::types::*;


#[derive(Debug)]
pub struct XRef {
    pub tpe: XRefType,
    pub map: BTreeMap<ObjNum, Record>,
    pub dict: Dict,
    pub size: ObjNum
}


#[derive(Debug)]
pub enum XRefType {
    Table,
    Stream(ObjRef)
}


#[derive(Debug, Clone, Copy)]
pub enum Record {
    Used { gen: ObjGen, offset: Offset },
    Compr { num: ObjNum, index: ObjGen },
    Free { gen: ObjGen, next: ObjNum }
}

impl Default for Record {
    fn default() -> Self {
        Record::Free { gen: 65535, next: 0 }
    }
}


pub trait Locator {
    fn locate(&self, objref: &ObjRef) -> Option<Record>;

    fn locate_offset(&self, objref: &ObjRef) -> Option<Offset> {
        if let Some(Record::Used{offset, ..}) = self.locate(objref) {
            Some(offset)
        } else {
            None
        }
    }
}

impl Locator for () {
    fn locate(&self, _objref: &ObjRef) -> Option<Record> {
        None
    }
}

impl Locator for XRef {
    fn locate(&self, objref: &ObjRef) -> Option<Record> {
        if objref.num >= self.size {
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
