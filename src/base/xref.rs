use super::*;
use super::types::*;

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
