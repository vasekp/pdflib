use super::bp::ByteProvider;
use super::cc::CharClass;

pub type Token = Vec<u8>;

pub trait Tokenizer: ByteProvider {
    fn read_token(&mut self) -> std::io::Result<Token> {
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
            CharClass::Space => {
                while self.next_if(|c| CharClass::of(c) == CharClass::Space).is_some() { }
                Ok(vec![b' '])
            },
            CharClass::Reg => {
                let mut ret = Vec::new();
                ret.push(c);
                while let Some(r) = self.next_if(|c| CharClass::of(c) == CharClass::Reg) {
                    ret.push(r);
                }
                Ok(ret)
            }
        }
    }

    fn read_token_nonempty(&mut self) -> std::io::Result<Token> {
        loop {
            let tk = self.read_token()?;
            if tk != b" " { return Ok(tk); }
        }
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
        assert_eq!(tkn.read_token().unwrap(), b" ");
        assert_eq!(tkn.read_token().unwrap(), b"<<");
        assert_eq!(tkn.read_token().unwrap(), b"g,");
        assert_eq!(tkn.read_token().unwrap(), b" ");
        assert_eq!(tkn.read_token().unwrap(), b" ");
        assert_eq!(tkn.read_token().unwrap(), b"n");
        assert!(tkn.read_token().is_err());

        let mut tkn = Cursor::new("A%1\rB%2\nC");
        assert_eq!(tkn.read_token().unwrap(), b"A");
        assert_eq!(tkn.read_token().unwrap(), b" ");
        assert_eq!(tkn.read_token().unwrap(), b" ");
        assert_eq!(tkn.read_token().unwrap(), b"B");
        assert_eq!(tkn.read_token().unwrap(), b" ");
        assert_eq!(tkn.read_token().unwrap(), b" ");
        assert_eq!(tkn.read_token().unwrap(), b"C");

        let mut tkn = Cursor::new("A%1\r %2\nB");
        assert_eq!(tkn.read_token_nonempty().unwrap(), b"A");
        assert_eq!(tkn.read_token_nonempty().unwrap(), b"B");
    }
}
