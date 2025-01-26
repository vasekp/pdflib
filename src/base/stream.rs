use super::*;
use super::types::*;

#[derive(Debug, PartialEq, Clone)]
pub struct Stream {
    pub dict: Dict,
    pub data: Data
}

#[derive(Debug, PartialEq, Clone)]
pub enum Data {
    Ref(Offset),
    Val(Vec<u8>)
}
