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
    Compr { num_within: ObjNum, index: ObjIndex },
    Free { gen: ObjGen, next: ObjNum }
}

impl Default for Record {
    fn default() -> Self {
        Record::Free { gen: 65535, next: 0 }
    }
}
