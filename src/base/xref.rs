use super::*;
use super::types::*;

#[derive(Debug)]
pub enum XRefType {
    Table,
    Stream(ObjRef)
}

#[derive(Debug)]
pub enum Record {
    Used { gen: ObjGen, offset: Offset },
    Free { gen: ObjGen, next: ObjNum },
    Compr { num: ObjNum, index: ObjGen }
}
