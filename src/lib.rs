use std::iter::Peekable;
use std::io::{Read, Cursor};
use std::fmt::{Debug, Formatter};

#[derive(Debug, PartialEq)]
enum CharClass {
    Space,
    Delim,
    Reg
}

impl CharClass {
    fn of(ch: u8) -> CharClass {
        match ch {
            b'\x00' | b'\x09' | b'\x0A' | b'\x0C' | b'\x0D' | b'\x20' => CharClass::Space,
            b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%' => CharClass::Delim,
            _ => CharClass::Reg
        }
    }
}

fn read_token<T: Iterator<Item = std::io::Result<u8>>>(iter: &mut Peekable<T>) -> std::io::Result<Vec<u8>> {
    let c = iter.next().ok_or(std::io::Error::from(std::io::ErrorKind::UnexpectedEof))??;
    match CharClass::of(c) {
        CharClass::Delim => {
            if (c == b'<' || c == b'>') && iter.next_if(|r| r.as_ref().is_ok_and(|c2| *c2 == c)).is_some() {
                Ok([c, c].into())
            } else if c == b'%' {
                while iter.next_if(|r| r.as_ref().is_ok_and(|c| *c != b'\n' && *c != b'\r')).is_some() { }
                Ok([b' '].into())
            } else {
                Ok([c].into())
            }
        },
        CharClass::Space => {
            while iter.next_if(|r| r.as_ref().is_ok_and(|c| CharClass::of(*c) == CharClass::Space)).is_some() { }
            Ok([b' '].into())
        },
        CharClass::Reg => {
            let mut ret = Vec::new();
            ret.push(c);
            while let Some(r) = iter.next_if(|r| r.as_ref().is_ok_and(|c| CharClass::of(*c) == CharClass::Reg)) {
                ret.push(r?);
            }
            Ok(ret)
        }
    }
}

fn read_token_nonempty<T: Iterator<Item = std::io::Result<u8>>>(iter: &mut Peekable<T>) -> std::io::Result<Vec<u8>> {
    loop {
        let tk = read_token(iter)?;
        if tk != b" " { return Ok(tk); }
    }
}

#[derive(Debug, PartialEq)]
struct Name(Vec<u8>);

#[derive(Debug, PartialEq)]
enum Object {
    Bool(bool),
    Number(Number),
    String(Vec<u8>),
    Name(Name),
    Array(Vec<Object>),
    Dict(Vec<(Name, Object)>),
    Stream(Vec<(Name, Object)>, Vec<u8>),
    Null
}

#[derive(Debug, PartialEq)]
enum Number {
    Int(i64),
    Real(f64)
}

fn to_number(tok: &[u8]) -> Result<Number, ()> {
    if tok.contains(&b'.') {
        Ok(Number::Real(std::str::from_utf8(tok)
            .map_err(|_| ())?
            .parse::<f64>()
            .map_err(|_| ())?))
    } else {
        Ok(Number::Int(std::str::from_utf8(tok)
            .map_err(|_| ())?
            .parse::<i64>()
            .map_err(|_| ())?))
    }
}

fn read_obj<T: Iterator<Item = std::io::Result<u8>>>(iter: &mut Peekable<T>) -> std::io::Result<Object> {
    match &read_token_nonempty(iter)?[..] {
        b"true" => Ok(Object::Bool(true)),
        b"false" => Ok(Object::Bool(false)),
        tk @ [b'0'..=b'9' | b'+' | b'-' | b'.', ..] => Ok(Object::Number(to_number(tk)
                .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "Malformed number"))?)),
        _ => todo!()
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
    fn test_tokenizer() {
        let input = "abc  <<g,%k\r\nn";
        let cur = Cursor::new(input);
        let mut bytes = cur.bytes().peekable();
        assert_eq!(read_token(&mut bytes).unwrap(), b"abc");
        assert_eq!(read_token(&mut bytes).unwrap(), b" ");
        assert_eq!(read_token(&mut bytes).unwrap(), b"<<");
        assert_eq!(read_token(&mut bytes).unwrap(), b"g,");
        assert_eq!(read_token(&mut bytes).unwrap(), b" ");
        assert_eq!(read_token(&mut bytes).unwrap(), b" ");
        assert_eq!(read_token(&mut bytes).unwrap(), b"n");
        assert!(read_token(&mut bytes).is_err());

        let input = "A%1\rB%2\nC";
        let cur = Cursor::new(input);
        let mut bytes = cur.bytes().peekable();
        assert_eq!(read_token(&mut bytes).unwrap(), b"A");
        assert_eq!(read_token(&mut bytes).unwrap(), b" ");
        assert_eq!(read_token(&mut bytes).unwrap(), b" ");
        assert_eq!(read_token(&mut bytes).unwrap(), b"B");
        assert_eq!(read_token(&mut bytes).unwrap(), b" ");
        assert_eq!(read_token(&mut bytes).unwrap(), b" ");
        assert_eq!(read_token(&mut bytes).unwrap(), b"C");

        let input = "A%1\r %2\nB";
        let cur = Cursor::new(input);
        let mut bytes = cur.bytes().peekable();
        assert_eq!(read_token_nonempty(&mut bytes).unwrap(), b"A");
        assert_eq!(read_token_nonempty(&mut bytes).unwrap(), b"B");
    }

    #[test]
    fn test_read_obj() {
        let input = "true false 123 +17 -98 0 34.5 -3.62 +123.6 4. -.002 0.0";
        let cur = Cursor::new(input);
        let mut bytes = cur.bytes().peekable();
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::Bool(true));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::Bool(false));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::Number(Number::Int(123)));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::Number(Number::Int(17)));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::Number(Number::Int(-98)));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::Number(Number::Int(0)));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::Number(Number::Real(34.5)));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::Number(Number::Real(-3.62)));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::Number(Number::Real(123.6)));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::Number(Number::Real(4.)));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::Number(Number::Real(-0.002)));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::Number(Number::Real(0.)));

        let cur = Cursor::new("++1 1..0 .1. 1_ 1a true");
        let mut bytes = cur.bytes().peekable();
        assert!(read_obj(&mut bytes).is_err());
        assert!(read_obj(&mut bytes).is_err());
        assert!(read_obj(&mut bytes).is_err());
        assert!(read_obj(&mut bytes).is_err());
        assert!(read_obj(&mut bytes).is_err());
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::Bool(true));
    }
}
