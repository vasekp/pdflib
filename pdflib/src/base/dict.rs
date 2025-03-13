use std::fmt::{Display, Formatter};

use super::name::Name;
use super::object::Object;

/// Dictionary objects (like `<< /Length 42 >>`).
#[derive(Debug, PartialEq, Clone, Default)]
pub struct Dict(Vec<(Name, Object)>);

impl Dict {
    /// Looks up for a value for a given [`Name`] key. If not present, returns a static reference 
    /// to [`Object::Null`].
    pub fn lookup(&self, key: &[u8]) -> &Object {
        self.0.iter()
            .find(|(name, _obj)| name == &key)
            .map(|(_name, obj)| obj)
            .unwrap_or(&Object::Null)
    }
}

impl From<Vec<(Name, Object)>> for Dict {
    fn from(vec: Vec<(Name, Object)>) -> Dict {
        Dict(vec)
    }
}

impl IntoIterator for Dict {
    type Item = (Name, Object);
    type IntoIter = <Vec<(Name, Object)> as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a Dict {
    type Item = &'a (Name, Object);
    type IntoIter = std::slice::Iter<'a, (Name, Object)>;

    fn into_iter(self: &'a Dict) -> Self::IntoIter {
        self.0.iter()
    }
}

impl Display for Dict {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("<< ")?;
        for (key, val) in &self.0 {
            write!(f, "{key} {val} ")?;
        }
        f.write_str(">>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base::*;

    #[test]
    fn test_dict() {
        let dict = Dict::from(vec![
            (Name::from(b"NKey"), Object::new_name(b"Nvalue")),
            (Name::from(b"IKey"), Object::Number(Number::Int(10))),
        ]);
        assert_eq!(dict.lookup(b"NKey"), &Object::new_name(b"Nvalue"));
        assert_eq!(dict.lookup(b"IKey"), &Object::Number(Number::Int(10)));
        assert_eq!(dict.lookup(b"Missing"), &Object::Null);
    }
}
