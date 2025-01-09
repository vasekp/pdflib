use super::*;
use super::types::*;

#[derive(Debug, PartialEq, Clone)]
pub struct Stream {
    pub dict: Dict,
    pub data: Data
}

#[derive(Debug, PartialEq, Clone)]
pub enum Data {
    Ref(IndirectData),
    Val(Vec<u8>)
}

#[derive(Debug, PartialEq, Default, Clone)]
pub struct IndirectData {
    pub offset: Offset,
    pub len: Option<u64>,
    pub filters: Vec<Name>,
    // TODO fparams: Option<Dict>
}
