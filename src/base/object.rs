use std::fmt::{Display, Formatter};

use super::name::Name;
use super::dict::Dict;
use super::number::Number;
use super::string::format_string;
use super::stream::Stream;

#[derive(Debug, PartialEq)]
pub enum Object {
    Bool(bool),
    Number(Number),
    String(Vec<u8>),
    Name(Name),
    Array(Vec<Object>),
    Dict(Dict),
    Stream(Stream),
    Ref(ObjRef),
    Null
}

impl Object {
    pub fn new_string(s: &str) -> Object {
        Object::String(s.bytes().collect())
    }

    pub fn new_name(s: &str) -> Object {
        Object::Name(Name::from(s))
    }
}

impl Display for Object {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Object::Bool(true) => f.write_str("true"),
            Object::Bool(false) => f.write_str("false"),
            Object::Number(Number::Int(x)) => write!(f, "{x}"),
            Object::Number(Number::Real(x)) => write!(f, "{x}"),
            Object::String(s) => format_string(f, s),
            Object::Name(name) => write!(f, "{}", name),
            Object::Array(arr) => {
                f.write_str("[ ")?;
                for obj in arr {
                    write!(f, "{obj} ")?;
                }
                f.write_str("]")
            },
            Object::Dict(dict) => write!(f, "{}", dict),
            Object::Stream(stm) => write!(f, "{} stream...", stm.dict),
            Object::Ref(ObjRef(num, gen)) => write!(f, "{num} {gen} R"),
            Object::Null => f.write_str("null")
        }
    }
}

#[derive(PartialEq, Debug, Clone, Copy)]
pub struct ObjRef(pub u64, pub u16);


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", Object::Number(Number::Real(-1.))), "-1");
        assert_eq!(format!("{}", Object::Number(Number::Real(0.0000000000000001))), "0.0000000000000001");
        assert_eq!(format!("{}", Object::new_string("")), "()");
        assert_eq!(format!("{}", Object::new_string("\0\r\n\\")), "(\\000\\r\\n\\\\)");
        assert_eq!(format!("{}", Object::new_string("()")), "(\\(\\))");
        assert_eq!(format!("{}", Object::new_string("a\nb c")), "(a\\nb c)");
        assert_eq!(format!("{}", Object::new_name(" A#/$*(%\n")), "/#20A#23#2F$*#28#25#0A");
        assert_eq!(format!("{}", Object::Array(vec![
                Object::Number(Number::Int(549)),
                Object::Number(Number::Real(3.14)),
                Object::Bool(false),
                Object::new_string("Ralph"),
                Object::new_name("SomeName")
        ])), "[ 549 3.14 false (Ralph) /SomeName ]");
        assert_eq!(format!("{}", Object::Array(vec![Object::Array(vec![Object::Bool(true)])])), "[ [ true ] ]");
        assert_eq!(format!("{}", Object::Dict(Dict(vec![
            (Name::from("Type"), Object::new_name("Example")),
            (Name::from("Subtype"), Object::new_name("DictionaryExample")),
            (Name::from("Version"), Object::Number(Number::Real(0.01))),
            (Name::from("IntegerItem"), Object::Number(Number::Int(12))),
            (Name::from("StringItem"), Object::new_string("a string")),
            (Name::from("Subdictionary"), Object::Dict(Dict(vec![
                (Name::from("Item1"), Object::Number(Number::Real(0.4))),
                (Name::from("Item2"), Object::Bool(true)),
                (Name::from("LastItem"), Object::new_string("not !")),
                (Name::from("VeryLastItem"), Object::new_string("OK"))
            ])))
        ]))), "<< /Type /Example /Subtype /DictionaryExample /Version 0.01 /IntegerItem 12 \
        /StringItem (a string) /Subdictionary << /Item1 0.4 /Item2 true /LastItem (not !) \
        /VeryLastItem (OK) >> >>");
        assert_eq!(format!("{}", Object::Dict(Dict(vec![
            (Name::from("Length"), Object::Ref(ObjRef(8, 0)))]))), "<< /Length 8 0 R >>");
    }
}
