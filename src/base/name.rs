use std::fmt::{Display, Debug, Formatter};

#[derive(PartialEq, Clone)]
pub struct Name(pub Vec<u8>);

impl From<&[u8]> for Name {
    fn from(s: &[u8]) -> Name {
        Name(s.to_owned())
    }
}

impl<const N: usize> From<&[u8; N]> for Name {
    fn from(s: &[u8; N]) -> Name {
        Name(s.to_vec())
    }
}

impl Display for Name {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        use crate::parser::cc::CharClass;
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

impl<T: AsRef<[u8]>> PartialEq<T> for Name {
    fn eq(&self, other: &T) -> bool {
        self.0 == other.as_ref()
    }
}
