use super::bp::ByteProvider;
use super::cc::CharClass;
use crate::base::Error;

pub type Token = Vec<u8>;

pub trait Tokenizer: ByteProvider {
    fn read_token(&mut self) -> std::io::Result<Token> {
        self.skip_ws()?;
        let c = self.next_or_eof()?;
        match CharClass::of(c) {
            CharClass::Delim => {
                if (c == b'<' || c == b'>') && self.next_if(|c2| c2 == c).is_some() {
                    Ok(vec![c, c])
                } else if c == b'%' {
                    while self.next_if(|c| c != b'\n' && c != b'\r').is_some() { }
                    Ok(vec![b' '])
                } else {
                    Ok(vec![c])
                }
            },
            CharClass::Reg => {
                let mut ret = Vec::new();
                ret.push(c);
                while let Some(r) = self.next_if(|c| CharClass::of(c) == CharClass::Reg) {
                    ret.push(r);
                }
                Ok(ret)
            },
            CharClass::Space => unreachable!()
        }
    }

    fn read_eol(&mut self) -> Result<(), Error> {
        match (self.next_if(|c| c == b'\r'), self.next_if(|c| c == b'\n')) {
            (None, None) => Err(Error::Parse("EOL expected but not found")),
            _ => Ok(())
        }
    }

    fn skip_ws(&mut self) -> std::io::Result<()> {
        while let Some(c) = self.next_if(|c| CharClass::of(c) == CharClass::Space || c == b'%') {
            if c == b'%' {
                while self.next_if(|c| c != b'\n' && c != b'\r').is_some() { }
            }
        }
        Ok(())
    }
}

impl<T: ByteProvider> Tokenizer for T { }


#[cfg(test)]
mod tests {
    #[test]
    fn test_tokenizer() {
        use super::*;
        use std::io::Cursor;

        let mut tkn = Cursor::new("abc  <<g,%k\r\nn");
        assert_eq!(tkn.read_token().unwrap(), b"abc");
        assert_eq!(tkn.read_token().unwrap(), b"<<");
        assert_eq!(tkn.read_token().unwrap(), b"g,");
        assert_eq!(tkn.read_token().unwrap(), b"n");
        assert!(tkn.read_token().is_err());

        let mut tkn = Cursor::new("A %1\r B%2\n\rC");
        assert_eq!(tkn.read_token().unwrap(), b"A");
        assert_eq!(tkn.read_token().unwrap(), b"B");
        assert_eq!(tkn.read_token().unwrap(), b"C");

        let mut tkn = Cursor::new("a\r\nb\nc\rd \ne\n\r");
        assert_eq!(tkn.read_token().unwrap(), b"a");
        assert!(tkn.read_eol().is_ok());
        assert_eq!(tkn.read_token().unwrap(), b"b");
        assert!(tkn.read_eol().is_ok());
        assert_eq!(tkn.read_token().unwrap(), b"c");
        assert!(tkn.read_eol().is_ok());
        assert_eq!(tkn.read_token().unwrap(), b"d");
        assert!(tkn.read_eol().is_err());
        assert_eq!(tkn.read_token().unwrap(), b"e");
        assert!(tkn.read_eol().is_ok());
        assert!(tkn.read_eol().is_ok());
        assert!(tkn.read_eol().is_err());
    }
}
