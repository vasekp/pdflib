use std::io::{BufRead, Cursor, Seek};

use crate::base::*;


pub trait ByteProvider: BufRead + Seek {
    fn peek(&mut self) -> Option<u8> {
        match self.fill_buf() {
            Ok(buf) => Some(buf[0]),
            _ => None
        }
    }

    fn next_or_eof(&mut self) -> std::io::Result<u8> {
        let buf = self.fill_buf()?;
        if !buf.is_empty() {
            let ret = buf[0];
            self.consume(1);
            Ok(ret)
        } else {
            Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof))
        }
    }

    fn next_if(&mut self, cond: impl FnOnce(u8) -> bool) -> Option<u8> {
        let buf = self.fill_buf().ok()?;
        if !buf.is_empty() && cond(buf[0]) {
            let ret = buf[0];
            self.consume(1);
            Some(ret)
        } else {
            None
        }
    }

    fn read_line(&mut self) -> std::io::Result<Vec<u8>> {
        let mut line = Vec::new();
        loop {
            let buf = match self.fill_buf() {
                Ok(buf) => buf,
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err)
            };
            if buf.is_empty() {
                if line.is_empty() {
                    return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
                } else {
                    break;
                }
            }
            match buf.iter().position(|c| *c == b'\n' || *c == b'\r') {
                Some(pos) => {
                    line.extend_from_slice(&buf[..pos]);
                    let crlf = buf[pos] == b'\r' && buf.len() > pos && buf[pos + 1] == b'\n';
                    self.consume(pos + if crlf { 2 } else { 1 });
                    break;
                },
                None => {
                    line.extend_from_slice(buf);
                    let len = buf.len();
                    self.consume(len);
                }
            }
        }
        Ok(line)
    }

    fn skip_past_eol(&mut self) -> std::io::Result<()> {
        loop {
            let buf = match self.fill_buf() {
                Ok(buf) => buf,
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err)
            };
            if buf.is_empty() {
                return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
            }
            match buf.iter().position(|c| *c == b'\n' || *c == b'\r') {
                Some(pos) => {
                    let crlf = buf[pos] == b'\r' && buf.len() > pos && buf[pos + 1] == b'\n';
                    self.consume(pos + if crlf { 2 } else { 1 });
                    return Ok(());
                },
                None => {
                    let len = buf.len();
                    self.consume(len);
                }
            }
        }
    }
}

impl<T: BufRead + Seek> ByteProvider for T { }


type Token = Vec<u8>;

struct Tokenizer<T: ByteProvider> {
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

    fn read_token_nonempty(&mut self) -> std::io::Result<Token> {
        loop {
            let tk = self.read_token()?;
            if tk != b" " { return Ok(tk); }
        }
    }

    fn new(bytes: T) -> Self {
        Self { bytes, stack: Vec::with_capacity(3) }
    }

    fn next(&mut self) -> std::io::Result<Token> {
        match self.stack.pop() {
            Some(tk) => Ok(tk),
            None => self.read_token_nonempty()
        }
    }

    fn unread(&mut self, tk: Token) {
        self.stack.push(tk);
    }

    fn bytes(&mut self) -> &mut T {
        assert!(self.stack.is_empty());
        &mut self.bytes
    }

    fn seek_to(&mut self, pos: u64) -> std::io::Result<()> {
        self.stack.clear();
        self.bytes.seek(std::io::SeekFrom::Start(pos)).map(|_| ())
    }

    fn pos(&mut self) -> std::io::Result<u64> {
        assert!(self.stack.is_empty());
        self.bytes.stream_position()
    }
}

impl<T: Into<String>> From<T> for Tokenizer<Cursor<String>> {
    fn from(input: T) -> Self {
        Tokenizer::new(Cursor::new(input.into()))
    }
}


pub struct Parser<T: ByteProvider> {
    tkn: Tokenizer<T>
}


impl<T: ByteProvider> Parser<T> {
    pub fn new(reader: T) -> Self {
        Self { tkn: Tokenizer::new(reader) }
    }

    pub fn seek_to(&mut self, pos: u64) -> std::io::Result<()> {
        self.tkn.seek_to(pos)
    }

    pub fn pos(&mut self) -> std::io::Result<u64> {
        self.tkn.pos()
    }

    pub fn read_obj(&mut self) -> Result<Object, Error> {
        let first = self.tkn.next()?;
        match &first[..] {
            b"true" => Ok(Object::Bool(true)),
            b"false" => Ok(Object::Bool(false)),
            b"null" => Ok(Object::Null),
            [b'0'..=b'9', ..] => {
                self.tkn.unread(first);
                self.read_number_or_indirect() },
            [b'+' | b'-' | b'.', ..] => {
                self.tkn.unread(first);
                self.read_number().map(Object::Number) },
            b"(" => self.read_lit_string(),
            b"<" => self.read_hex_string(),
            b"/" => self.read_name().map(Object::Name),
            b"[" => self.read_array(),
            b"<<" => self.read_dict(),
            b"R" => Err(Error::Parse("Unexcepted token: R")),
            _ => todo!("{:?}", std::str::from_utf8(&first))
        }
    }

    fn read_number(&mut self) -> Result<Number, Error> {
        Self::to_number_inner(&self.tkn.next()?)
            .map_err(|_| Error::Parse("Malformed number"))
    }

    fn read_number_or_indirect(&mut self) -> Result<Object, Error> {
        let num = Self::to_number_inner(&self.tkn.next()?)
            .map_err(|_| Error::Parse("Malformed number"))?;
        let Number::Int(num) = num else {
            return Ok(Object::Number(num))
        };
        assert!(num >= 0);
        let gen_tk = self.tkn.next()?;
        match Self::to_number_inner(&gen_tk) {
            Ok(Number::Int(gen)) if gen <= u16::MAX as i64 => {
                let r_tk = self.tkn.next()?;
                if r_tk == b"R" {
                    return Ok(Object::Ref(ObjRef(num as u64, gen as u16)));
                } else {
                    self.tkn.unread(r_tk);
                    self.tkn.unread(gen_tk);
                }
            },
            _ => self.tkn.unread(gen_tk)
        }
        Ok(Object::Number(Number::Int(num)))
    }

    fn parse<U: std::str::FromStr>(bstr: &[u8]) -> Result<U, Error> {
        std::str::from_utf8(bstr)
            .map_err(|_| Error::Parse("parse error"))?
            .parse::<U>()
            .map_err(|_| Error::Parse("parse error"))
    }

    fn to_number_inner(tok: &Token) -> Result<Number, Error> {
        if tok.contains(&b'e') || tok.contains(&b'E') {
            return Err(Error::Parse("parse error"))
        }
        if tok.contains(&b'.') {
            Ok(Number::Real(Self::parse::<f64>(tok)?))
        } else {
            Ok(Number::Int(Self::parse::<i64>(tok)?))
        }
    }

    fn read_lit_string(&mut self) -> Result<Object, Error> {
        let mut ret = Vec::new();
        let mut parens = 0;
        let bytes = self.tkn.bytes();
        loop {
            match bytes.next_or_eof()? {
                b'\\' => {
                    let c = match bytes.next_or_eof()? {
                        b'n' => b'\x0a',
                        b'r' => b'\x0d',
                        b't' => b'\x09',
                        b'b' => b'\x08',
                        b'f' => b'\x0c',
                        c @ (b'(' | b')' | b'\\') => c,
                        d1 @ (b'0' ..= b'7') => {
                            let d1 = d1 - b'0';
                            let d2 = bytes.next_if(|c| (b'0'..=b'7').contains(&c)).map(|c| c - b'0');
                            let d3 = bytes.next_if(|c| (b'0'..=b'7').contains(&c)).map(|c| c - b'0');
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
                    bytes.next_if(|c| c == b'\n');
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
        let bytes = self.tkn.bytes();
        loop {
            let c = bytes.next_or_eof()?;
            let dig = match c {
                b'0'..=b'9' => c - b'0',
                b'a'..=b'f' => c - b'a' + 10,
                b'A'..=b'F' => c - b'A' + 10,
                b'>' => {
                    if let Some(d) = msd { ret.push(d << 4); }
                    break;
                },
                d if CharClass::of(d) == CharClass::Space => continue,
                _ => return Err(Error::Parse("Malformed hex string"))
            };
            match msd {
                None => msd = Some(dig),
                Some(d) => { ret.push((d << 4) | dig); msd = None; }
            }
        }
        Ok(Object::String(ret))
    }

    fn read_name(&mut self) -> Result<Name, Error> {
        match self.tkn.bytes().peek() {
            Some(c) if CharClass::of(c) != CharClass::Reg => return Ok(Name(Vec::new())),
            None => return Ok(Name(Vec::new())),
            _ => ()
        };
        let tk = self.tkn.next()?;
        if !tk.contains(&b'#') {
            return Ok(Name(tk));
        }
        let mut parts = tk.split(|c| *c == b'#');
        let mut ret: Vec<u8> = parts.next().unwrap().into(); // nonemptiness checked in contains()
        let hex = |c| match c {
            b'0'..=b'9' => Ok(c - b'0'),
            b'a'..=b'f' => Ok(c - b'a' + 10),
            b'A'..=b'F' => Ok(c - b'A' + 10),
            _ => Err(Error::Parse("Malformed name"))
        };
        for part in parts {
            if part.len() < 2 {
                return Err(Error::Parse("Malformed name"));
            }
            if &part[0..=1] == b"00" {
                return Err(Error::Parse("Illegal name (contains #00)"));
            }
            let d1 = hex(part[0])?;
            let d2 = hex(part[1])?;
            ret.push((d1 << 4) + d2);
            ret.extend_from_slice(&part[2..]);
        }
        Ok(Name(ret))
    }

    fn read_array(&mut self) -> Result<Object, Error> {
        let mut vec = Vec::new();
        loop {
            let tk = self.tkn.next()?;
            if tk == b"]" { break; }
            self.tkn.unread(tk);
            vec.push(self.read_obj()?);
        }
        Ok(Object::Array(vec))
    }

    fn read_dict(&mut self) -> Result<Object, Error> {
        let mut dict = Vec::new();
        loop {
            let key = match &self.tkn.next()?[..] {
                b">>" => break,
                b"/" => self.read_name()?,
                _ => return Err(Error::Parse("Malformed dictionary"))
            };
            let value = self.read_obj()?;
            dict.push((key, value));
        }
        Ok(Object::Dict(Dict(dict)))
    }

    pub fn entrypoint(&mut self) -> Result<XRef, Error> {
        let bytes = self.tkn.bytes();
        let len = bytes.seek(std::io::SeekFrom::End(0))?;
        let buf_size = std::cmp::min(len, 1024);

        // Read last 1024 bytes
        bytes.seek(std::io::SeekFrom::End(-(buf_size as i64)))?;
        // TODO: use read_buf_exact when stabilized
        let mut data = vec![0; buf_size as usize];
        bytes.read_exact(&mut data)?;

        // Find "startxref<EOL>number<EOL>"
        let sxref = data.windows(9)
            .rposition(|w| w == b"startxref")
            .ok_or(Error::Parse("startxref not found"))?;
        let mut cur = Cursor::new(&data[sxref..]);
        cur.skip_past_eol()?;
        let sxref = Self::parse::<u64>(&ByteProvider::read_line(&mut cur)?)
            .map_err(|_| Error::Parse("malformed startxref"))?;
        bytes.seek(std::io::SeekFrom::Start(sxref))?;

        // Read xref table. TODO: XRef stream
        if self.tkn.next()? != b"xref" {
            return Err(Error::Parse("xref not found"));
        }
        let bytes = self.tkn.bytes();
        bytes.skip_past_eol()?;
        let mut table = std::collections::BTreeMap::new();
        let err = || Error::Parse("malformed xref table");
        loop {
            let line = ByteProvider::read_line(bytes)?.trim_ascii_end().to_owned();
            if line == b"trailer" { break; }
            let index = line.iter().position(|c| *c == b' ').ok_or_else(err)?;
            let start = Self::parse::<u64>(&line[..index]).map_err(|_| err())?;
            let size = Self::parse::<u64>(&line[(index+1)..]).map_err(|_| err())?;
            /*bytes.seek(std::io::SeekFrom::Current(20 * (size as i64)))?;*/
            let mut line = [0u8; 20];
            for num in start..(start+size) {
                bytes.read_exact(&mut line)?;
                if line[10] != b' ' || line[16] != b' ' {
                    return Err(err());
                }
                let v = Self::parse::<u64>(&line[0..10]).map_err(|_| err())?;
                let gen = Self::parse::<u16>(&line[11..16]).map_err(|_| err())?;
                match line[17] {
                    b'n' => table.insert(num, Record::Used{gen, offset: v}),
                    b'f' => table.insert(num, Record::Free{gen, next: v}),
                    _ => return Err(err())
                };
            }
        }
        let trailer = match self.read_obj()? {
            Object::Dict(dict) => dict,
            _ => return Err(Error::Parse("malformed trailer"))
        };
        Ok(XRef{table, trailer})
    }

    pub fn read_obj_indirect(&mut self, num: u64, gen: u16) -> Result<Object, Error> {
        let Number::Int(n2) = self.read_number()? else { return Err(Error::Parse("unexpected token")) };
        if n2 != num as i64 { return Err(Error::Parse("number does not match")) };
        let Number::Int(g2) = self.read_number()? else { return Err(Error::Parse("unexpected token")) };
        if g2 != gen as i64 { return Err(Error::Parse("number does not match")) };
        if self.tkn.read_token_nonempty()? == b"obj" {
            self.read_obj()
        } else {
            Err(Error::Parse("unexpected token"))
        }
    }
}

impl<T: Into<String>> From<T> for Parser<Cursor<String>> {
    fn from(input: T) -> Self {
        Parser { tkn: Tokenizer::from(input) }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_line() {
        let mut bytes = Cursor::new("line 1\nline 2\rline 3\r\nline 4\n\rline 5");
        assert_eq!(ByteProvider::read_line(&mut bytes).unwrap(), b"line 1");
        assert_eq!(ByteProvider::read_line(&mut bytes).unwrap(), b"line 2");
        assert_eq!(ByteProvider::read_line(&mut bytes).unwrap(), b"line 3");
        assert_eq!(ByteProvider::read_line(&mut bytes).unwrap(), b"line 4");
        assert_eq!(ByteProvider::read_line(&mut bytes).unwrap(), b"");
        assert_eq!(ByteProvider::read_line(&mut bytes).unwrap(), b"line 5");
        assert!(ByteProvider::read_line(&mut bytes).is_err());

        let mut bytes = Cursor::new("line 1\nline 2\rline 3\r\nline 4\n\rline 5");
        bytes.skip_past_eol().unwrap();
        bytes.skip_past_eol().unwrap();
        bytes.skip_past_eol().unwrap();
        bytes.skip_past_eol().unwrap();
        assert_eq!(ByteProvider::read_line(&mut bytes).unwrap(), b"");
        assert!(ByteProvider::skip_past_eol(&mut bytes).is_err());
    }

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

    #[test]
    fn test_read_obj() {
        let mut parser = Parser::from("true false null 123 +17 -98 0 34.5 -3.62 +123.6 4. -.002 0.0");
        assert_eq!(parser.read_obj().unwrap(), Object::Bool(true));
        assert_eq!(parser.read_obj().unwrap(), Object::Bool(false));
        assert_eq!(parser.read_obj().unwrap(), Object::Null);
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Int(123)));
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Int(17)));
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Int(-98)));
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Int(0)));
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Real(34.5)));
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Real(-3.62)));
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Real(123.6)));
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Real(4.)));
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Real(-0.002)));
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Real(0.)));

        let mut parser = Parser::from("++1 1..0 .1. 1_ 1a 16#FFFE . 6.023E23 true");
        assert!(parser.read_obj().is_err());
        assert!(parser.read_obj().is_err());
        assert!(parser.read_obj().is_err());
        assert!(parser.read_obj().is_err());
        assert!(parser.read_obj().is_err());
        assert!(parser.read_obj().is_err());
        assert!(parser.read_obj().is_err());
        assert!(parser.read_obj().is_err());
        assert_eq!(parser.read_obj().unwrap(), Object::Bool(true));
    }

    #[test]
    fn test_read_lit_string() {
        let mut parser = Parser::from("(string) (new
line) (parens() (*!&}^%etc).) () ((0)) (()");
        assert_eq!(parser.read_obj().unwrap(), Object::new_string("string"));
        assert_eq!(parser.read_obj().unwrap(), Object::new_string("new\nline"));
        assert_eq!(parser.read_obj().unwrap(), Object::new_string("parens() (*!&}^%etc)."));
        assert_eq!(parser.read_obj().unwrap(), Object::new_string(""));
        assert_eq!(parser.read_obj().unwrap(), Object::new_string("(0)"));
        assert!(parser.read_obj().is_err());

        let mut parser = Parser::from("(These \\
two strings \\
are the same.) (These two strings are the same.)");
        assert_eq!(parser.read_obj().unwrap(), parser.read_obj().unwrap());

        let mut parser = Parser::from("(1
) (2\\n) (3\\r) (4\\r\\n)");
        assert_eq!(parser.read_obj().unwrap(), Object::new_string("1\n"));
        assert_eq!(parser.read_obj().unwrap(), Object::new_string("2\n"));
        assert_eq!(parser.read_obj().unwrap(), Object::new_string("3\r"));
        assert_eq!(parser.read_obj().unwrap(), Object::new_string("4\r\n"));

        let mut parser = Parser::from("(1
) (2\n) (3\r) (4\r\n)");
        assert_eq!(parser.read_obj().unwrap(), Object::new_string("1\n"));
        assert_eq!(parser.read_obj().unwrap(), Object::new_string("2\n"));
        assert_eq!(parser.read_obj().unwrap(), Object::new_string("3\n"));
        assert_eq!(parser.read_obj().unwrap(), Object::new_string("4\n"));

        let mut parser = Parser::from("(\\157cta\\154) (\\500) (\\0053\\053\\53) (\\53x)");
        assert_eq!(parser.read_obj().unwrap(), Object::new_string("octal"));
        assert_eq!(parser.read_obj().unwrap(), Object::new_string("@"));
        assert_eq!(parser.read_obj().unwrap(), Object::new_string("\x053++"));
        assert_eq!(parser.read_obj().unwrap(), Object::new_string("+x"));
    }

    #[test]
    fn test_read_hex_string() {
        let mut parser = Parser::from("<4E6F762073686D6F7A206B6120706F702E> <901FA3> <901fa>");
        assert_eq!(parser.read_obj().unwrap(), Object::new_string("Nov shmoz ka pop."));
        assert_eq!(parser.read_obj().unwrap(), Object::String(vec![0x90, 0x1F, 0xA3]));
        assert_eq!(parser.read_obj().unwrap(), Object::String(vec![0x90, 0x1F, 0xA0]));

        let mut parser = Parser::from("<61\r\n62> <61%comment\n>");
        assert_eq!(parser.read_obj().unwrap(), Object::new_string("ab"));
        assert!(parser.read_obj().is_err());
    }

    #[test]
    fn test_read_name() {
        let mut parser = Parser::from("/Name1 /A;Name_With-Various***Characters? /1.2 /$$ /@pattern
            /.notdef /Lime#20Green /paired#28#29parentheses /The_Key_of_F#23_Minor /A#42");
        assert_eq!(parser.read_obj().unwrap(), Object::new_name("Name1"));
        assert_eq!(parser.read_obj().unwrap(), Object::new_name("A;Name_With-Various***Characters?"));
        assert_eq!(parser.read_obj().unwrap(), Object::new_name("1.2"));
        assert_eq!(parser.read_obj().unwrap(), Object::new_name("$$"));
        assert_eq!(parser.read_obj().unwrap(), Object::new_name("@pattern"));
        assert_eq!(parser.read_obj().unwrap(), Object::new_name(".notdef"));
        assert_eq!(parser.read_obj().unwrap(), Object::new_name("Lime Green"));
        assert_eq!(parser.read_obj().unwrap(), Object::new_name("paired()parentheses"));
        assert_eq!(parser.read_obj().unwrap(), Object::new_name("The_Key_of_F#_Minor"));
        assert_eq!(parser.read_obj().unwrap(), Object::new_name("AB"));

        let mut parser = Parser::from("//%\n1 /ok /invalid#00byte /#0x /#0 true");
        assert_eq!(parser.read_obj().unwrap(), Object::new_name(""));
        assert_eq!(parser.read_obj().unwrap(), Object::new_name(""));
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Int(1)));
        assert_eq!(parser.read_obj().unwrap(), Object::new_name("ok"));
        assert!(parser.read_obj().is_err());
        assert!(parser.read_obj().is_err());
        assert!(parser.read_obj().is_err());
        assert_eq!(parser.read_obj().unwrap(), Object::Bool(true));
    }

    #[test]
    fn test_read_array() {
        let mut parser = Parser::from("[549 3.14 false (Ralph) /SomeName] [ %\n ] [false%]");
        assert_eq!(parser.read_obj().unwrap(), Object::Array(vec![
                Object::Number(Number::Int(549)),
                Object::Number(Number::Real(3.14)),
                Object::Bool(false),
                Object::new_string("Ralph"),
                Object::new_name("SomeName")
        ]));
        assert_eq!(parser.read_obj().unwrap(), Object::Array(Vec::new()));
        assert!(parser.read_obj().is_err());
    }

    #[test]
    fn test_read_dict() {
        let mut parser = Parser::from("<</Type /Example
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
        assert_eq!(parser.read_obj().unwrap(), Object::Dict(Dict(vec![
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
        let mut parser = Parser::from("<</Length 8 0 R>>");
        assert_eq!(parser.read_obj().unwrap(), Object::Dict(Dict(vec![
            (Name::from("Length"), Object::Ref(ObjRef(8, 0)))
        ])));

        let mut parser = Parser::from("1 2 3 R 4 R");
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Int(1)));
        assert_eq!(parser.read_obj().unwrap(), Object::Ref(ObjRef(2, 3)));
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Int(4)));
        assert!(parser.read_obj().is_err());
    }
}
