use crate::base::*;
use super::xref::XRef;

pub enum TLO {
    IndirObject(ObjRef, Object),
    Stream(ObjRef, Stream),
    XRef(XRef)
}

pub struct Stream {
    dict: Dict,
    data: Data
}

pub enum Data {
    Ref(u64),
    Val(Vec<u8>)
}
