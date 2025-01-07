use super::*;

#[derive(Debug)]
pub enum XRefType {
    Table,
    Stream(ObjRef)
}

#[derive(Debug)]
pub enum Record {
    Used { gen: u16, offset: u64 },
    Free { gen: u16, next: u64 },
    Compr { num: u64, index: u16 }
}
