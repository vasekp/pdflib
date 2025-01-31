use super::*;

pub trait Locator {
    fn locate(&self, objref: &ObjRef) -> Option<Record>;
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
