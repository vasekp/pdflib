use std::fmt::{Display, Debug, Formatter};

#[derive(Debug, PartialEq)]
pub enum Object {
    Bool(bool),
    Number(Number),
    String(Vec<u8>),
    Name(Name),
    Array(Vec<Object>),
    Dict(Dict),
    //Stream(Vec<(Name, Object)>, Vec<u8>),
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
            Object::Ref(ObjRef(num, gen)) => write!(f, "{num} {gen} R"),
            Object::Null => f.write_str("null")
        }
    }
}

//TODO: literal / hex heuristics
fn format_string(f: &mut Formatter<'_>, s: &Vec<u8>) -> std::fmt::Result {
    f.write_str("(")?;
    for c in s {
        match c {
            b'\x0a' => f.write_str("\\n"),
            b'\x0d' => f.write_str("\\r"),
            b'\x09' => f.write_str("\\t"),
            b'\x08' => f.write_str("\\b"),
            b'\x0c' => f.write_str("\\f"),
            b'(' => f.write_str("\\("),
            b')' => f.write_str("\\)"),
            b'\\' => f.write_str("\\\\"),
            b'\x20' ..= b'\x7E' => write!(f, "{}", *c as char),
            _ => write!(f, "\\{c:03o}")
        }?
    }
    f.write_str(")")
}


#[derive(Debug, PartialEq)]
pub enum Number {
    Int(i64),
    Real(f64)
}


#[derive(PartialEq)]
pub struct Name(pub Vec<u8>);

impl From<&str> for Name {
    fn from(s: &str) -> Name {
        Name(s.bytes().collect())
    }
}

impl Display for Name {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("/")?;
        for c in &self.0 {
            if (0x21..=0x7E).contains(c) && matches!(CharClass::of(*c), CharClass::Reg) && *c != b'#' {
                write!(f, "{}", *c as char)?
            } else {
                write!(f, "#{:02X}", c)?
            }
        }
        Ok(())
    }
}

impl Debug for Name {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self)
    }
}

#[derive(PartialEq, Debug)]
pub struct ObjRef(pub u64, pub u16);


#[derive(Debug, PartialEq)]
pub struct Dict(pub Vec<(Name, Object)>);

impl Display for Dict {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("<< ")?;
        for (key, val) in &self.0 {
            write!(f, "{key} {val} ")?;
        }
        f.write_str(">>")
    }
}


#[derive(Debug, PartialEq)]
pub(crate) enum CharClass {
    Space,
    Delim,
    Reg
}

impl CharClass {
    pub fn of(ch: u8) -> CharClass {
        match ch {
            b'\x00' | b'\x09' | b'\x0A' | b'\x0C' | b'\x0D' | b'\x20' => CharClass::Space,
            b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%' => CharClass::Delim,
            _ => CharClass::Reg
        }
    }
}


#[derive(Debug)]
pub enum Error {
    IO(std::io::Error),
    Parse(&'static str)
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Error {
        Error::IO(err)
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cc() {
        assert_eq!(CharClass::of(b'\0'), CharClass::Space);
        assert_eq!(CharClass::of(b'\r'), CharClass::Space);
        assert_eq!(CharClass::of(b'\n'), CharClass::Space);
        assert_eq!(CharClass::of(b'\t'), CharClass::Space);
        assert_eq!(CharClass::of(b' '), CharClass::Space);
        assert_eq!(CharClass::of(b'('), CharClass::Delim);
        assert_eq!(CharClass::of(b')'), CharClass::Delim);
        assert_eq!(CharClass::of(b'{'), CharClass::Delim);
        assert_eq!(CharClass::of(b'}'), CharClass::Delim);
        assert_eq!(CharClass::of(b'['), CharClass::Delim);
        assert_eq!(CharClass::of(b']'), CharClass::Delim);
        assert_eq!(CharClass::of(b'<'), CharClass::Delim);
        assert_eq!(CharClass::of(b'>'), CharClass::Delim);
        assert_eq!(CharClass::of(b'/'), CharClass::Delim);
        assert_eq!(CharClass::of(b'%'), CharClass::Delim);
        assert_eq!(CharClass::of(b'a'), CharClass::Reg);
        assert_eq!(CharClass::of(b'\\'), CharClass::Reg);
        assert_eq!(CharClass::of(b'\''), CharClass::Reg);
        assert_eq!(CharClass::of(b'\"'), CharClass::Reg);
        assert_eq!(CharClass::of(b'\x08'), CharClass::Reg);
    }

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
