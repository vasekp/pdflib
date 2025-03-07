use std::collections::BTreeMap;

use super::*;
use super::types::*;

/// A cross-reference table, or a table section, or a cross-reference stream.
#[derive(Debug)]
pub struct XRef {
    /// The format in which this table section appears or should appear in a file.
    pub tpe: XRefType,
    /// The mapping itself.
    ///
    /// NB that for accessing records one should generally use the [`Locator`] interface, which can 
    /// handle traversal through the cross-reference history.
    pub map: BTreeMap<ObjNum, Record>,
    /// The trailer dictionary (for [`XRefType::Table`]) or the cross-reference stream dictionary 
    /// (for [`XRefType::Stream`]).
    pub dict: Dict,
    /// The `/Size` entry in the dictionary, for convenience.
    pub size: ObjNum
}

/// The format of a cross-reference table section.
#[derive(Debug)]
pub enum XRefType {
    /// Classical table (`xref ... trailer << ... >>`)
    Table,
    /// A cross-reference stream (`<< /Type/XRef ... >> stream ... endstream`)
    Stream(ObjRef)
}

impl XRef {
    /// Merge two cross-reference table sections into one by filling in missing entries by those 
    /// from `prev`. (An entry present in `self` always has preference.) All other fields of `prev`,
    /// most notably its trailer dictionary, are ignored.
    pub fn merge_prev(&mut self, mut prev: XRef) {
        prev.map.append(&mut self.map);
        self.map = prev.map;
    }
}


/// A single record in a cross-reference table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Record {
    /// An uncompressed object (`n` entry).
    Used {
        /// The generation number.
        gen: ObjGen,
        /// Location of the object in PDF file (w.r.t. `%PDF`).
        offset: Offset,
    },
    /// An object number marked as free (`f` entry).
    Free {
        /// The generation number to be used if this object number is reused for a new object.
        gen: ObjGen,
        /// The next number in the free object list, or zero if `gen` is 65535 (`u16::MAX`).
        next: ObjNum,
    },
    /// An object which is stored compressed within an object stream. The generation number of both 
    /// the compressed object and the containing stream is zero.
    Compr {
        /// The object number of the object stream (generation number is always zero).
        num_within: ObjNum,
        /// 0-based order of this compressed object within the object stream.
        index: ObjIndex,
    },
}

impl Default for Record {
    /// Returns `Record::Free { gen: 65535, next: 0 }.`
    fn default() -> Self {
        Record::Free { gen: 65535, next: 0 }
    }
}
