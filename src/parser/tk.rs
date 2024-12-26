use std::io::Cursor;
use super::bp::ByteProvider;
use super::cc::CharClass;

pub type Token = Vec<u8>;

pub struct Tokenizer<T: ByteProvider> {
    bytes: T,
    stack: Vec<Token>
}


impl<T: ByteProvider> Tokenizer<T> {
    fn read_token(&mut self) -> std::io::Result<Token> {
        let c = self.bytes.next_or_eof()?;
        match CharClass::of(c) {
            CharClass::Delim => {
                if (c == b'<' || c == b'>') && self.bytes.next_if(|c2| c2 == c).is_some() {
                    Ok(vec![c, c])
                } else if c == b'%' {
                    while self.bytes.next_if(|c| c != b'\n' && c != b'\r').is_some() { }
                    Ok(vec![b' '])
                } else {
                    Ok(vec![c])
                }
            },
            CharClass::Space => {
                while self.bytes.next_if(|c| CharClass::of(c) == CharClass::Space).is_some() { }
                Ok(vec![b' '])
            },
            CharClass::Reg => {
                let mut ret = Vec::new();
                ret.push(c);
                while let Some(r) = self.bytes.next_if(|c| CharClass::of(c) == CharClass::Reg) {
                    ret.push(r);
                }
                Ok(ret)
            }
        }
    }

    pub fn read_token_nonempty(&mut self) -> std::io::Result<Token> {
        loop {
            let tk = self.read_token()?;
            if tk != b" " { return Ok(tk); }
        }
    }

    pub fn new(bytes: T) -> Self {
        Self { bytes, stack: Vec::with_capacity(3) }
    }

    pub fn next(&mut self) -> std::io::Result<Token> {
        match self.stack.pop() {
            Some(tk) => Ok(tk),
            None => self.read_token_nonempty()
        }
    }

    pub fn unread(&mut self, tk: Token) {
        self.stack.push(tk);
    }

    pub fn bytes(&mut self) -> &mut T {
        assert!(self.stack.is_empty());
        &mut self.bytes
    }

    pub fn seek_to(&mut self, pos: u64) -> std::io::Result<()> {
        self.stack.clear();
        self.bytes.seek(std::io::SeekFrom::Start(pos)).map(|_| ())
    }

    pub fn pos(&mut self) -> std::io::Result<u64> {
        assert!(self.stack.is_empty());
        self.bytes.stream_position()
    }
}

impl<T: Into<String>> From<T> for Tokenizer<Cursor<String>> {
    fn from(input: T) -> Self {
        Tokenizer::new(Cursor::new(input.into()))
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenizer() {
        let mut tkn = Tokenizer::from("abc  <<g,%k\r\nn");
        assert_eq!(tkn.read_token().unwrap(), b"abc");
        assert_eq!(tkn.read_token().unwrap(), b" ");
        assert_eq!(tkn.read_token().unwrap(), b"<<");
        assert_eq!(tkn.read_token().unwrap(), b"g,");
        assert_eq!(tkn.read_token().unwrap(), b" ");
        assert_eq!(tkn.read_token().unwrap(), b" ");
        assert_eq!(tkn.read_token().unwrap(), b"n");
        assert!(tkn.read_token().is_err());

        let mut tkn = Tokenizer::from("A%1\rB%2\nC");
        assert_eq!(tkn.read_token().unwrap(), b"A");
        assert_eq!(tkn.read_token().unwrap(), b" ");
        assert_eq!(tkn.read_token().unwrap(), b" ");
        assert_eq!(tkn.read_token().unwrap(), b"B");
        assert_eq!(tkn.read_token().unwrap(), b" ");
        assert_eq!(tkn.read_token().unwrap(), b" ");
        assert_eq!(tkn.read_token().unwrap(), b"C");

        let mut tkn = Tokenizer::from("A%1\r %2\nB");
        assert_eq!(tkn.read_token_nonempty().unwrap(), b"A");
        assert_eq!(tkn.read_token_nonempty().unwrap(), b"B");
    }
}
