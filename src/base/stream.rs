use super::dict::Dict;
use super::types::*;

#[derive(Debug, PartialEq)]
pub struct Stream {
    pub dict: Dict,
    pub data: Data
}

#[derive(Debug, PartialEq)]
pub enum Data {
    Ref(Offset),
    Val(Vec<u8>)
}
