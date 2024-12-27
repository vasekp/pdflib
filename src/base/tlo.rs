use crate::base::*;
use super::xref::XRef;

pub enum TLO {
    IndirObject(ObjRef, Object),
    XRef(XRef)
}
