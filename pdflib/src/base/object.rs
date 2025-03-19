use std::fmt::{Display, Formatter};

use super::name::Name;
use super::dict::Dict;
use super::number::Number;
use super::string::format_string;
use super::stream::{self, Stream, StreamData};
use super::types::*;

/// The base type of all PDF objects.
///
/// This can come in two flavours, see [`stream::StreamData`]. For PDF reading applications, the 
/// appropriate variant is `Data = Offset`, which is facilitated by the type alias [`Object`].
#[derive(Debug, PartialEq, Clone)]
pub enum BaseObject<Data: StreamData> {
    /// Bool (`true` or `false`)
    Bool(bool),
    /// Numbers (integer or real)
    Number(Number),
    /// Strings.
    ///
    /// No distinction is made whether this was literal or hex-encoded in the source.
    String(Vec<u8>),
    /// Name (like `/Length`)
    Name(Name),
    /// Array (`[1 2 3]`)
    Array(Vec<Self>),
    /// Dictionary (`<< /Root 1 0 R >>`)
    Dict(Dict),
    /// Stream (`<< ... >> stream ... endstream`)
    Stream(Stream<Data>),
    /// Indirect object reference (`3 0 R`)
    Ref(ObjRef),
    /// Null object (`null`). Also used as a fall-back where the specification says.
    Null
}

impl<Data: StreamData> BaseObject<Data> {
    /// A utility method to create [`Self::String`] from a byte slice.
    pub fn new_string(s: &[u8]) -> Self {
        Self::String(s.to_owned())
    }

    /// A utility method to create [`Self::Name`] from a byte slice. Don't pass the initial 
    /// `'/'` unless the name is actually supposed to start with `#2F`.
    pub fn new_name(s: &[u8]) -> Self {
        Self::Name(Name::from(s))
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            &BaseObject::Bool(val) => Some(val),
            _ => None
        }
    }

    pub fn as_string(&self) -> Option<&Vec<u8>> {
        match self {
            Self::String(val) => Some(val),
            _ => None
        }
    }

    pub fn as_name(&self) -> Option<&Name> {
        match self {
            Self::Name(val) => Some(val),
            _ => None
        }
    }

    pub fn as_array(&self) -> Option<&Vec<Self>> {
        match self {
            Self::Array(val) => Some(val),
            _ => None
        }
    }

    pub fn as_dict(&self) -> Option<&Dict> {
        match self {
            Self::Dict(val) => Some(val),
            _ => None
        }
    }

    pub fn as_stream(&self) -> Option<&Stream<Data>> {
        match self {
            Self::Stream(val) => Some(val),
            _ => None
        }
    }

    pub fn as_objref(&self) -> Option<&ObjRef> {
        match self {
            Self::Ref(val) => Some(val),
            _ => None
        }
    }

    pub fn into_string(self) -> Option<Vec<u8>> {
        match self {
            Self::String(val) => Some(val),
            _ => None
        }
    }

    pub fn into_name(self) -> Option<Name> {
        match self {
            Self::Name(val) => Some(val),
            _ => None
        }
    }

    pub fn into_array(self) -> Option<Vec<Self>> {
        match self {
            Self::Array(val) => Some(val),
            _ => None
        }
    }

    pub fn into_dict(self) -> Option<Dict> {
        match self {
            Self::Dict(val) => Some(val),
            _ => None
        }
    }

    pub fn into_stream(self) -> Option<Stream<Data>> {
        match self {
            Self::Stream(val) => Some(val),
            _ => None
        }
    }

    pub fn into_objref(self) -> Option<ObjRef> {
        match self {
            Self::Ref(val) => Some(val),
            _ => None
        }
    }

    /// For `Self::Number(Number::Int(number))`, extracts the `number` and casts it into the 
    /// required type. Returns `None` both for other types of objects and for value too large for the 
    /// type `T`.
    pub fn num_value<T: TryFrom<i64>>(&self) -> Option<T> {
        match self {
            &Self::Number(Number::Int(num)) => num.try_into().ok(),
            _ => None
        }
    }
}

impl<Data: StreamData> Display for BaseObject<Data> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bool(true) => f.write_str("true"),
            Self::Bool(false) => f.write_str("false"),
            Self::Number(Number::Int(x)) => write!(f, "{x}"),
            Self::Number(Number::Real(x)) => write!(f, "{x}"),
            Self::String(s) => format_string(f, s),
            Self::Name(name) => write!(f, "{}", name),
            Self::Array(arr) => {
                f.write_str("[ ")?;
                for obj in arr {
                    write!(f, "{obj} ")?;
                }
                f.write_str("]")
            },
            Self::Dict(dict) => write!(f, "{}", dict),
            Self::Stream(stm) => write!(f, "{} [stream]", stm.dict),
            Self::Ref(ObjRef{num, gen}) => write!(f, "{num} {gen} R"),
            Self::Null => f.write_str("null")
        }
    }
}

/// A shorthand for [`BaseObject<Offset>`].
///
/// In this variant the [`data`](Stream::data) field of the `Self::Stream` variant is an offset 
/// into the PDF file. This is the logical default for all read-only scenarios.
pub type Object = BaseObject<stream::ByRef>;

/// An indirect object reference.
#[derive(PartialEq, Debug, Clone, Copy)]
pub struct ObjRef {
    pub num: ObjNum,
    pub gen: ObjGen
}

impl Display for ObjRef {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.num, self.gen)
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", Object::Number(Number::Real(-1.))), "-1");
        assert_eq!(format!("{}", Object::Number(Number::Real(0.0000000000000001))), "0.0000000000000001");
        assert_eq!(format!("{}", Object::new_string(b"")), "()");
        assert_eq!(format!("{}", Object::new_string(b"\0\r\n\\")), "(\\000\\r\\n\\\\)");
        assert_eq!(format!("{}", Object::new_string(b"()")), "(\\(\\))");
        assert_eq!(format!("{}", Object::new_string(b"a\nb c")), "(a\\nb c)");
        assert_eq!(format!("{}", Object::new_name(b" A#/$*(%\n")), "/#20A#23#2F$*#28#25#0A");
        assert_eq!(format!("{}", Object::Array(vec![
                Object::Number(Number::Int(549)),
                #[allow(clippy::approx_constant)]
                Object::Number(Number::Real(3.14)),
                Object::Bool(false),
                Object::new_string(b"Ralph"),
                Object::new_name(b"SomeName")
        ])), "[ 549 3.14 false (Ralph) /SomeName ]");
        assert_eq!(format!("{}", Object::Array(vec![Object::Array(vec![Object::Bool(true)])])), "[ [ true ] ]");
        assert_eq!(format!("{}", Object::Dict(Dict::from(vec![
            (Name::from(b"Type"), Object::new_name(b"Example")),
            (Name::from(b"Subtype"), Object::new_name(b"DictionaryExample")),
            (Name::from(b"Version"), Object::Number(Number::Real(0.01))),
            (Name::from(b"IntegerItem"), Object::Number(Number::Int(12))),
            (Name::from(b"StringItem"), Object::new_string(b"a string")),
            (Name::from(b"Subdictionary"), Object::Dict(Dict::from(vec![
                (Name::from(b"Item1"), Object::Number(Number::Real(0.4))),
                (Name::from(b"Item2"), Object::Bool(true)),
                (Name::from(b"LastItem"), Object::new_string(b"not !")),
                (Name::from(b"VeryLastItem"), Object::new_string(b"OK"))
            ])))
        ]))), "<< /Type /Example /Subtype /DictionaryExample /Version 0.01 /IntegerItem 12 \
        /StringItem (a string) /Subdictionary << /Item1 0.4 /Item2 true /LastItem (not !) \
        /VeryLastItem (OK) >> >>");
        assert_eq!(format!("{}", Object::Dict(Dict::from(vec![
            (Name::from(b"Length"), Object::Ref(ObjRef{num: 8, gen: 0}))]))), "<< /Length 8 0 R >>");
    }
}
