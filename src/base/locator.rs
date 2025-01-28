use super::*;
use super::types::*;

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
