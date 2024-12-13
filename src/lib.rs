use std::iter::Peekable;
use std::io::{Read, Cursor};

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
    }
}
