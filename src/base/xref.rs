use super::dict::Dict;

pub struct XRef {
    pub table: std::collections::BTreeMap<u64, Record>,
    pub trailer: Dict
}

pub enum Record {
    Used { gen: u16, offset: u64 },
    Free { gen: u16, next: u64},
    Compr { num: u64, index: u16 }
}