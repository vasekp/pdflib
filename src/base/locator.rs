use super::*;

/// This trait requires a single method, [`Locator::locate()`]. Implementors use this to resolve object 
/// numbers to their entries in the cross-reference table.
///
/// A null implementation is provided for `()`.
pub trait Locator {
    /// Look up a given [`ObjRef`]. This should perform a lookup in the cross-reference table and 
    /// check if the generation number agrees with the expectation. In the case of mismatch this 
    /// method should return `Some(Record::default())`.
    fn locate(&self, objref: &ObjRef) -> Option<Record>;
}

impl Locator for () {
    fn locate(&self, _objref: &ObjRef) -> Option<Record> {
        None
    }
}

impl Locator for XRef {
    /// Returns `Some(record)` if the record is found in this table section and the generation 
    /// number agrees with `objref.gen`. Returns `Some(Record::default())` in cases of mismatch or 
    /// when the requested object number is out of bounds given by `/Size`, even if such record 
    /// exists. Returns `None` if no record is present.
    ///
    /// Recursive lookup through the update history is provided by other implementors, like the opaque 
    /// type returned by [`reader::FullReader::base_locator()`](crate::reader::FullReader::base_locator()).
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
