use std::fmt::{Display, Formatter};

use super::name::Name;
use super::object::Object;

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
