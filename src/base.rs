use std::fmt::{Display, Debug, Formatter};

#[derive(Debug, PartialEq)]
pub enum Object {
    Bool(bool),
    Number(Number),
    String(Vec<u8>),
    Name(Name),
    Array(Vec<Object>),
    Dict(Vec<(Name, Object)>),
    //Stream(Vec<(Name, Object)>, Vec<u8>),
    Indirect(ObjRef),
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
pub struct ObjRef(pub u64, pub u32);


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
    fn test_name_display() {
        assert_eq!(format!("{}", Name::from(" A#/$*(%\n")), "/#20A#23#2F$*#28#25#0A");
    }
}
