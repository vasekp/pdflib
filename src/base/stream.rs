use super::dict::Dict;

#[derive(Debug, PartialEq)]
pub struct Stream {
    pub dict: Dict,
    pub data: Data
}

#[derive(Debug, PartialEq)]
pub enum Data {
    Ref(u64),
    Val(Vec<u8>)
}
