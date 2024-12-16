use std::iter::Peekable;
use std::io::{Read, Cursor};
use std::fmt::{Debug, Formatter};

use crate::base::*;

struct ByteIterator<T: Iterator<Item = std::io::Result<u8>>>(Peekable<T>);

trait ByteIteratorT {
    fn next_or_eof(&mut self) -> std::io::Result<u8>;
    fn next_if(&mut self, cond: impl FnOnce(u8) -> bool) -> Option<u8>;
    fn peek(&mut self) -> Option<u8>;
}

impl<T: Iterator<Item = std::io::Result<u8>>> ByteIterator<T> {
    fn from(iter: T) -> ByteIterator<T> {
        ByteIterator(iter.peekable())
    }
}

impl<T: Iterator<Item = std::io::Result<u8>>> ByteIteratorT for ByteIterator<T> {
    fn next_or_eof(&mut self) -> std::io::Result<u8> {
        self.0.next().ok_or(std::io::Error::from(std::io::ErrorKind::UnexpectedEof))?
    }

    fn next_if(&mut self, cond: impl FnOnce(u8) -> bool) -> Option<u8> {
        self.0.next_if(|r| r.as_ref().is_ok_and(|c| cond(*c)))
            .transpose()
            .unwrap() // is_ok checked within next_if
    }

    fn peek(&mut self) -> Option<u8> {
        match self.0.peek() {
            Some(Ok(c)) => Some(*c),
            _ => None
        }
    }
}

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

fn read_token(iter: &mut impl ByteIteratorT) -> std::io::Result<Vec<u8>> {
    let c = iter.next_or_eof()?;
    match CharClass::of(c) {
        CharClass::Delim => {
            if (c == b'<' || c == b'>') && iter.next_if(|c2| c2 == c).is_some() {
                Ok([c, c].into())
            } else if c == b'%' {
                while iter.next_if(|c| c != b'\n' && c != b'\r').is_some() { }
                Ok([b' '].into())
            } else {
                Ok([c].into())
            }
        },
        CharClass::Space => {
            while iter.next_if(|c| CharClass::of(c) == CharClass::Space).is_some() { }
            Ok([b' '].into())
        },
        CharClass::Reg => {
            let mut ret = Vec::new();
            ret.push(c);
            while let Some(r) = iter.next_if(|c| CharClass::of(c) == CharClass::Reg) {
                ret.push(r);
            }
            Ok(ret)
        }
    }
}

fn read_token_nonempty(iter: &mut impl ByteIteratorT) -> std::io::Result<Vec<u8>> {
    loop {
        let tk = read_token(iter)?;
        if tk != b" " { return Ok(tk); }
    }
}

fn to_number(tok: &[u8]) -> Result<Number, ()> {
    if tok.contains(&b'e') || tok.contains(&b'E') {
        return Err(());
    }
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

fn read_obj(iter: &mut impl ByteIteratorT) -> std::io::Result<Object> {
    let first = read_token_nonempty(iter)?;
    read_obj_inner(iter, first)
}

fn read_obj_inner(iter: &mut impl ByteIteratorT, token: Vec<u8>) -> std::io::Result<Object> {
    match &token[..] {
        b"true" => Ok(Object::Bool(true)),
        b"false" => Ok(Object::Bool(false)),
        tk @ [b'0'..=b'9' | b'+' | b'-' | b'.', ..] => Ok(Object::Number(to_number(tk)
                .map_err(|_| std::io::Error::other("Malformed number"))?)),
        b"(" => read_lit_string(iter),
        b"<" => read_hex_string(iter),
        b"/" => read_name(iter),
        b"[" => read_array(iter),
        tk => todo!("{:?}", std::str::from_utf8(tk))
    }
}

fn read_lit_string(iter: &mut impl ByteIteratorT) -> std::io::Result<Object> {
    let mut ret = Vec::new();
    let mut parens = 0;
    loop {
        match iter.next_or_eof()? {
            b'\\' => {
                let c = match iter.next_or_eof()? {
                    b'n' => b'\x0a',
                    b'r' => b'\x0d',
                    b't' => b'\x09',
                    b'b' => b'\x08',
                    b'f' => b'\x0c',
                    c @ (b'(' | b')' | b'\\') => c,
                    d1 @ (b'0' ..= b'7') => {
                        let d1 = d1 - b'0';
                        let d2 = iter.next_if(|c| c >= b'0' && c <= b'7').map(|c| c - b'0');
                        let d3 = iter.next_if(|c| c >= b'0' && c <= b'7').map(|c| c - b'0');
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
                iter.next_if(|c| c == b'\n');
                ret.push(b'\n');
            },
            c => {
                if c == b'(' { parens = parens + 1; }
                if c == b')' {
                    if parens == 0 { break; } else { parens = parens - 1; }
                }
                ret.push(c);
            }
        }
    }
    Ok(Object::String(ret))
}

fn read_hex_string(iter: &mut impl ByteIteratorT) -> std::io::Result<Object> {
    let mut msd = None;
    let mut ret = Vec::new();
    loop {
        let c = iter.next_or_eof()?;
        let dig = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            b'>' => {
                if let Some(d) = msd { ret.push(d << 4); }
                break;
            },
            d if CharClass::of(d) == CharClass::Space => continue,
            _ => return Err(std::io::Error::other("Malformed hex string"))
        };
        match msd {
            None => msd = Some(dig),
            Some(d) => { ret.push((d << 4) | dig); msd = None; }
        }
    }
    Ok(Object::String(ret))
}

fn read_name(iter: &mut impl ByteIteratorT) -> std::io::Result<Object> {
    match iter.peek() {
        Some(c) if CharClass::of(c) != CharClass::Reg => return Ok(Object::Name(Name(Vec::new()))),
        None => return Ok(Object::Name(Name(Vec::new()))),
        _ => ()
    };
    let tk = read_token_nonempty(iter)?;
    if !tk.contains(&b'#') {
        return Ok(Object::Name(Name(tk)));
    }
    let mut parts = tk.split(|c| *c == b'#');
    let mut ret: Vec<u8> = parts.next().unwrap().into(); // nonemptiness checked in contains()
    for part in parts {
        if part.len() < 2 || !part[0].is_ascii_hexdigit() || !part[1].is_ascii_hexdigit() {
            return Err(std::io::Error::other("Malformed name"));
        }
        if &part[0..=1] == b"00" {
            return Err(std::io::Error::other("Illegal name (contains #00)"));
        }
        ret.push(u8::from_str_radix(std::str::from_utf8(&part[0..=1]).unwrap(), 16).unwrap()); // valdity of both checked
        ret.extend_from_slice(&part[2..]);
    }
    Ok(Object::Name(Name(ret)))
}

fn read_array(iter: &mut impl ByteIteratorT) -> std::io::Result<Object> {
    let mut vec = Vec::new();
    loop {
        let tk = read_token_nonempty(iter)?;
        if tk == b"]" { break; }
        vec.push(read_obj_inner(iter, tk)?);
    }
    Ok(Object::Array(vec))
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
        let mut bytes = ByteIterator::from(cur.bytes());
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
        let mut bytes = ByteIterator::from(cur.bytes());
        assert_eq!(read_token(&mut bytes).unwrap(), b"A");
        assert_eq!(read_token(&mut bytes).unwrap(), b" ");
        assert_eq!(read_token(&mut bytes).unwrap(), b" ");
        assert_eq!(read_token(&mut bytes).unwrap(), b"B");
        assert_eq!(read_token(&mut bytes).unwrap(), b" ");
        assert_eq!(read_token(&mut bytes).unwrap(), b" ");
        assert_eq!(read_token(&mut bytes).unwrap(), b"C");

        let input = "A%1\r %2\nB";
        let cur = Cursor::new(input);
        let mut bytes = ByteIterator::from(cur.bytes());
        assert_eq!(read_token_nonempty(&mut bytes).unwrap(), b"A");
        assert_eq!(read_token_nonempty(&mut bytes).unwrap(), b"B");
    }

    #[test]
    fn test_read_obj() {
        let input = "true false 123 +17 -98 0 34.5 -3.62 +123.6 4. -.002 0.0";
        let cur = Cursor::new(input);
        let mut bytes = ByteIterator::from(cur.bytes());
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

        let cur = Cursor::new("++1 1..0 .1. 1_ 1a 16#FFFE . 6.023E23 true");
        let mut bytes = ByteIterator::from(cur.bytes());
        assert!(read_obj(&mut bytes).is_err());
        assert!(read_obj(&mut bytes).is_err());
        assert!(read_obj(&mut bytes).is_err());
        assert!(read_obj(&mut bytes).is_err());
        assert!(read_obj(&mut bytes).is_err());
        assert!(read_obj(&mut bytes).is_err());
        assert!(read_obj(&mut bytes).is_err());
        assert!(read_obj(&mut bytes).is_err());
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::Bool(true));
    }

    #[test]
    fn test_read_lit_string() {
        let cur = Cursor::new("(string) (new
line) (parens() (*!&}^%etc).) () ((0)) (()");
        let mut bytes = ByteIterator::from(cur.bytes());
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_string("string"));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_string("new\nline"));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_string("parens() (*!&}^%etc)."));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_string(""));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_string("(0)"));
        assert!(read_obj(&mut bytes).is_err());

        let cur = Cursor::new("(These \\
two strings \\
are the same.) (These two strings are the same.)");
        let mut bytes = ByteIterator::from(cur.bytes());
        assert_eq!(read_obj(&mut bytes).unwrap(), read_obj(&mut bytes).unwrap());

        let cur = Cursor::new("(1
) (2\\n) (3\\r) (4\\r\\n)");
        let mut bytes = ByteIterator::from(cur.bytes());
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_string("1\n"));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_string("2\n"));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_string("3\r"));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_string("4\r\n"));

        let cur = Cursor::new("(1
) (2\n) (3\r) (4\r\n)");
        let mut bytes = ByteIterator::from(cur.bytes());
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_string("1\n"));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_string("2\n"));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_string("3\n"));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_string("4\n"));

        let cur = Cursor::new("(\\157cta\\154) (\\500) (\\0053\\053\\53) (\\53x)");
        let mut bytes = ByteIterator::from(cur.bytes());
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_string("octal"));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_string("@"));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_string("\x053++"));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_string("+x"));
    }

    #[test]
    fn test_read_hex_string() {
        let cur = Cursor::new("<4E6F762073686D6F7A206B6120706F702E> <901FA3> <901fa>");
        let mut bytes = ByteIterator::from(cur.bytes());
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_string("Nov shmoz ka pop."));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::String([0x90, 0x1F, 0xA3].into()));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::String([0x90, 0x1F, 0xA0].into()));

        let cur = Cursor::new("<61\r\n62> <61%comment\n>");
        let mut bytes = ByteIterator::from(cur.bytes());
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_string("ab"));
        assert!(read_obj(&mut bytes).is_err());
    }

    #[test]
    fn test_read_name() {
        let cur = Cursor::new("/Name1 /A;Name_With-Various***Characters? /1.2 /$$ /@pattern
            /.notdef /Lime#20Green /paired#28#29parentheses /The_Key_of_F#23_Minor /A#42");
        let mut bytes = ByteIterator::from(cur.bytes());
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_name("Name1"));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_name("A;Name_With-Various***Characters?"));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_name("1.2"));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_name("$$"));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_name("@pattern"));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_name(".notdef"));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_name("Lime Green"));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_name("paired()parentheses"));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_name("The_Key_of_F#_Minor"));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_name("AB"));

        let cur = Cursor::new("//%\n1 /ok /invalid#00byte /#0x /#0 true");
        let mut bytes = ByteIterator::from(cur.bytes());
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_name(""));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_name(""));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::Number(Number::Int(1)));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::new_name("ok"));
        assert!(read_obj(&mut bytes).is_err());
        assert!(read_obj(&mut bytes).is_err());
        assert!(read_obj(&mut bytes).is_err());
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::Bool(true));
    }

    #[test]
    fn test_read_array() {
        let cur = Cursor::new("[549 3.14 false (Ralph) /SomeName] [ %\n ] [false%]");
        let mut bytes = ByteIterator::from(cur.bytes());
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::Array([
                Object::Number(Number::Int(549)),
                Object::Number(Number::Real(3.14)),
                Object::Bool(false),
                Object::new_string("Ralph"),
                Object::new_name("SomeName")
        ].into()));
        assert_eq!(read_obj(&mut bytes).unwrap(), Object::Array(Vec::new()));
        assert!(read_obj(&mut bytes).is_err());
    }
}
