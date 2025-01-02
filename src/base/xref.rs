use super::*;

pub struct XRef {
    pub table: std::collections::BTreeMap<u64, Record>,
    pub size: u64,
    pub trailer: Dict,
    pub tpe: XRefType
}

pub enum XRefType {
    Table,
    Stream(ObjRef)
}

pub enum Record {
    Used { gen: u16, offset: u64 },
    Free { gen: u16, next: u64},
    Compr { num: u64, index: u16 }
}
