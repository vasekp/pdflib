use std::io::{BufRead, Cursor};

use crate::base::*;
use crate::utils;

use super::bp::ByteProvider;
use super::cc::CharClass;
use super::tk::*;

pub struct ObjParser<T: BufRead> {
    reader: T,
    stack: Vec<Token>
}

impl<T: BufRead> ObjParser<T> {
    pub fn new(reader: T) -> Self {
        Self { reader, stack: Default::default() }
    }

    fn next_token(&mut self) -> Result<Token, Error> {
        match self.stack.pop() {
            Some(tk) => Ok(tk),
            None => self.reader.read_token_nonempty()
                .map_err(Error::from)
        }
    }

    pub fn read_obj(reader: &mut T) -> Result<Object, Error> {
        let tk = reader.read_token_nonempty()?;
        if matches!(tk[..], [b'0'..=b'9' | b'+' | b'-' | b'.', ..]) {
            tk.try_into().map(Object::Number)
        } else {
            let mut op = ObjParser::new(reader);
            op.stack.push(tk);
            op.read_obj_inner()
        }
    }

    fn read_obj_inner(&mut self) -> Result<Object, Error> {
        let tk = self.next_token()?;
        match &tk[..] {
            b"true" => Ok(Object::Bool(true)),
            b"false" => Ok(Object::Bool(false)),
            b"null" => Ok(Object::Null),
            [b'1'..=b'9', ..] => {
                self.stack.push(tk);
                self.read_number_or_indirect()
            },
            [b'+' | b'-' | b'0' | b'.', ..]
                => tk.try_into().map(Object::Number),
            b"(" => self.read_lit_string(),
            b"<" => self.read_hex_string(),
            b"/" => self.read_name().map(Object::Name),
            b"[" => self.read_array(),
            b"<<" => self.read_dict(),
            _ => Err(Error::Parse("unexcepted token")),
        }
    }

    fn read_number_or_indirect(&mut self) -> Result<Object, Error> {
        let num = self.next_token()?.try_into()?;
        let Number::Int(num) = num else {
            return Ok(Object::Number(num))
        };
        let gen_tk = self.next_token()?;
        match utils::parse_int_strict(&gen_tk) {
            Some(gen) => {
                let r_tk = self.next_token()?;
                if r_tk == b"R" {
                    let num = num.try_into().unwrap(); // num already checked to start with 1..=9 and i64 fits
                    return Ok(Object::Ref(ObjRef{num, gen}));
                } else {
                    self.stack.push(r_tk);
                    self.stack.push(gen_tk);
                }
            },
            None => self.stack.push(gen_tk)
        }
        Ok(Object::Number(Number::Int(num)))
    }

    fn read_lit_string(&mut self) -> Result<Object, Error> {
        let mut ret = Vec::new();
        let mut parens = 0;
        loop {
            match self.reader.next_or_eof()? {
                b'\\' => {
                    let c = match self.reader.next_or_eof()? {
                        b'n' => b'\x0a',
                        b'r' => b'\x0d',
                        b't' => b'\x09',
                        b'b' => b'\x08',
                        b'f' => b'\x0c',
                        c @ (b'(' | b')' | b'\\') => c,
                        c @ (b'0' ..= b'7') => {
                            let d1 = c - b'0';
                            let d2 = self.reader.next_if(|c| (b'0'..=b'7').contains(&c)).map(|c| c - b'0');
                            let d3 = self.reader.next_if(|c| (b'0'..=b'7').contains(&c)).map(|c| c - b'0');
                            match (d2, d3) {
                                (Some(d2), Some(d3)) => (d1 << 6) + (d2 << 3) + d3,
                                (Some(d2), None) => (d1 << 3) + d2,
                                (None, None) => d1,
                                _ => unreachable!()
                            }
                        },
                        _ => continue
                    };
                    ret.push(c);
                },
                b'\r' => {
                    self.reader.next_if(|c| c == b'\n');
                    ret.push(b'\n');
                },
                c => {
                    if c == b'(' { parens += 1; }
                    if c == b')' {
                        if parens == 0 { break; } else { parens -= 1; }
                    }
                    ret.push(c);
                }
            }
        }
        Ok(Object::String(ret))
    }

    fn read_hex_string(&mut self) -> Result<Object, Error> {
        let mut msd = None;
        let mut ret = Vec::new();
        loop {
            let tk = self.next_token()?;
            if tk == b">" { break; }
            for c in tk {
                let dig = utils::hex_value(c).ok_or(Error::Parse("malformed hex string"))?;
                match msd {
                    None => msd = Some(dig),
                    Some(d) => { ret.push((d << 4) | dig); msd = None; }
                }
            }
        }
        if let Some(d) = msd { ret.push(d << 4); }
        Ok(Object::String(ret))
    }

    fn read_name(&mut self) -> Result<Name, Error> {
        match self.reader.peek() {
            Some(c) if CharClass::of(c) != CharClass::Reg => return Ok(Name(Vec::new())),
            None => return Ok(Name(Vec::new())),
            _ => ()
        };
        let tk = self.next_token()?;
        if !tk.contains(&b'#') {
            return Ok(Name(tk));
        }
        let mut parts = tk.split(|c| *c == b'#');
        let mut ret: Vec<u8> = parts.next().unwrap().into(); // nonemptiness checked in contains()
        for part in parts {
            if part.len() < 2 {
                return Err(Error::Parse("malformed name"));
            }
            if &part[0..=1] == b"00" {
                return Err(Error::Parse("illegal name (contains #00)"));
            }
            let d1 = utils::hex_value(part[0]).ok_or(Error::Parse("malformed name"))?;
            let d2 = utils::hex_value(part[1]).ok_or(Error::Parse("malformed name"))?;
            ret.push((d1 << 4) + d2);
            ret.extend_from_slice(&part[2..]);
        }
        Ok(Name(ret))
    }

    fn read_array(&mut self) -> Result<Object, Error> {
        let mut vec = Vec::new();
        loop {
            let tk = self.next_token()?;
            if tk == b"]" { break; }
            self.stack.push(tk);
            vec.push(self.read_obj_inner()?);
        }
        Ok(Object::Array(vec))
    }

    fn read_dict(&mut self) -> Result<Object, Error> {
        let mut dict = Vec::new();
        loop {
            let key = match &self.next_token()?[..] {
                b">>" => break,
                b"/" => self.read_name()?,
                _ => return Err(Error::Parse("malformed dictionary"))
            };
            let value = self.read_obj_inner()?;
            dict.push((key, value));
        }
        Ok(Object::Dict(Dict(dict)))
    }
}

#[cfg(debug_assertions)]
impl<T: BufRead> Drop for ObjParser<T> {
    fn drop(&mut self) {
        for tk in &self.stack {
            log::error!("Dropping stacked token: {}", std::str::from_utf8(tk).unwrap())
        }
    }
}

impl TryFrom<Token> for Number {
    type Error = Error;

    fn try_from(tk: Token) -> Result<Number, Error> {
        let err = Error::Parse("malformed number");
        if tk.contains(&b'e') || tk.contains(&b'E') {
            return Err(err)
        }
        if tk.contains(&b'.') {
            Ok(Number::Real(utils::parse_num(&tk).ok_or(err)?))
        } else {
            Ok(Number::Int(utils::parse_num(&tk).ok_or(err)?))
        }
    }
}

impl<T: Into<String>> From<T> for ObjParser<Cursor<String>> {
    fn from(input: T) -> Self {
        ObjParser::new(Cursor::new(input.into()))
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_obj() {
        let mut parser = ObjParser::from("true false null 123 +17 -98 0 00987 34.5 -3.62 +123.6 4. -.002 0.0 009.87");
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Bool(true));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Bool(false));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Null);
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Number(Number::Int(123)));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Number(Number::Int(17)));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Number(Number::Int(-98)));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Number(Number::Int(0)));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Number(Number::Int(987)));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Number(Number::Real(34.5)));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Number(Number::Real(-3.62)));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Number(Number::Real(123.6)));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Number(Number::Real(4.)));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Number(Number::Real(-0.002)));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Number(Number::Real(0.)));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Number(Number::Real(9.87)));

        let mut parser = ObjParser::from("9223372036854775807 9223372036854775808");
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Number(Number::Int(9223372036854775807)));
        assert!(parser.read_obj_inner().is_err());

        let mut parser = ObjParser::from("++1 1..0 .1. 1_ 1a 16#FFFE . 6.023E23 true");
        assert!(parser.read_obj_inner().is_err());
        assert!(parser.read_obj_inner().is_err());
        assert!(parser.read_obj_inner().is_err());
        assert!(parser.read_obj_inner().is_err());
        assert!(parser.read_obj_inner().is_err());
        assert!(parser.read_obj_inner().is_err());
        assert!(parser.read_obj_inner().is_err());
        assert!(parser.read_obj_inner().is_err());
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Bool(true));
    }

    #[test]
    fn test_read_lit_string() {
        let mut parser = ObjParser::from("(string) (new
line) (parens() (*!&}^%etc).) () ((0)) (()");
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_string("string"));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_string("new\nline"));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_string("parens() (*!&}^%etc)."));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_string(""));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_string("(0)"));
        assert!(parser.read_obj_inner().is_err());

        let mut parser = ObjParser::from("(These \\
two strings \\
are the same.) (These two strings are the same.)");
        assert_eq!(parser.read_obj_inner().unwrap(), parser.read_obj_inner().unwrap());

        let mut parser = ObjParser::from("(1
) (2\\n) (3\\r) (4\\r\\n)");
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_string("1\n"));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_string("2\n"));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_string("3\r"));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_string("4\r\n"));

        let mut parser = ObjParser::from("(1
) (2\n) (3\r) (4\r\n)");
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_string("1\n"));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_string("2\n"));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_string("3\n"));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_string("4\n"));

        let mut parser = ObjParser::from("(\\157cta\\154) (\\500) (\\0053\\053\\53) (\\53x)");
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_string("octal"));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_string("@"));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_string("\x053++"));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_string("+x"));
    }

    #[test]
    fn test_read_hex_string() {
        let mut parser = ObjParser::from("<4E6F762073686D6F7A206B6120706F702E> <901FA3> <901fa>");
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_string("Nov shmoz ka pop."));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::String(vec![0x90, 0x1F, 0xA3]));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::String(vec![0x90, 0x1F, 0xA0]));

        let mut parser = ObjParser::from("<61\r\n6 2> <61%comment\n> <61%unterminated>");
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_string("ab"));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_string("a"));
        assert!(parser.read_obj_inner().is_err());
    }

    #[test]
    fn test_read_name() {
        let mut parser = ObjParser::from("/Name1 /A;Name_With-Various***Characters? /1.2 /$$ /@pattern
            /.notdef /Lime#20Green /paired#28#29parentheses /The_Key_of_F#23_Minor /A#42");
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_name("Name1"));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_name("A;Name_With-Various***Characters?"));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_name("1.2"));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_name("$$"));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_name("@pattern"));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_name(".notdef"));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_name("Lime Green"));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_name("paired()parentheses"));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_name("The_Key_of_F#_Minor"));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_name("AB"));

        let mut parser = ObjParser::from("//%\n1 /ok /invalid#00byte /#0x /#0 true");
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_name(""));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_name(""));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Number(Number::Int(1)));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::new_name("ok"));
        assert!(parser.read_obj_inner().is_err());
        assert!(parser.read_obj_inner().is_err());
        assert!(parser.read_obj_inner().is_err());
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Bool(true));
    }

    #[test]
    fn test_read_array() {
        let mut parser = ObjParser::from("[549 3.14 false (Ralph) /SomeName] [ %\n ] [false%]");
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Array(vec![
                Object::Number(Number::Int(549)),
                Object::Number(Number::Real(3.14)),
                Object::Bool(false),
                Object::new_string("Ralph"),
                Object::new_name("SomeName")
        ]));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Array(Vec::new()));
        assert!(parser.read_obj_inner().is_err());
    }

    #[test]
    fn test_read_dict() {
        let mut parser = ObjParser::from("<</Type /Example
    /Subtype /DictionaryExample
    /Version 0.01
    /IntegerItem 12
    /StringItem (a string)
    /Subdictionary <<
        /Item1 0.4
        /Item2 true
        /LastItem (not !)
        /VeryLastItem (OK)
        >>
    >>");
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Dict(Dict(vec![
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
        ])));
    }

    #[test]
    fn test_read_indirect() {
        let mut parser = ObjParser::from("<</Length 8 0 R>>");
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Dict(Dict(vec![
            (Name::from("Length"), Object::Ref(ObjRef{num: 8, gen: 0}))
        ])));

        let mut parser = ObjParser::from("1 2 3 R 4 R");
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Number(Number::Int(1)));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Ref(ObjRef{num: 2, gen: 3}));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Number(Number::Int(4)));
        assert!(parser.read_obj_inner().is_err());

        let mut parser = ObjParser::from("0 0 R");
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Number(Number::Int(0)));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Number(Number::Int(0)));
        assert!(parser.read_obj_inner().is_err());

        let mut parser = ObjParser::from("01 0 R");
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Number(Number::Int(1)));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Number(Number::Int(0)));
        assert!(parser.read_obj_inner().is_err());

        let mut parser = ObjParser::from("1 01 R");
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Number(Number::Int(1)));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Number(Number::Int(1)));
        assert!(parser.read_obj_inner().is_err());

        let mut parser = ObjParser::from("1 +1 R");
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Number(Number::Int(1)));
        assert_eq!(parser.read_obj_inner().unwrap(), Object::Number(Number::Int(1)));
        assert!(parser.read_obj_inner().is_err());
    }
}
