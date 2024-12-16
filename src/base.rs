#[derive(Debug, PartialEq)]
pub enum Object {
    Bool(bool),
    Number(Number),
    String(Vec<u8>),
    Name(Name),
    Array(Vec<Object>),
    Dict(Vec<(Name, Object)>),
    Stream(Vec<(Name, Object)>, Vec<u8>),
    Null
}

impl Object {
    pub fn new_string(s: &str) -> Object {
        Object::String(s.bytes().collect())
    }

    pub fn new_name(s: &str) -> Object {
        Object::Name(Name(s.bytes().collect()))
    }
}

#[derive(Debug, PartialEq)]
pub enum Number {
    Int(i64),
    Real(f64)
}

#[derive(Debug, PartialEq)]
pub struct Name(pub Vec<u8>);
