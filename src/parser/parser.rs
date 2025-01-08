use std::io::{Cursor, Seek, Read};

use crate::base::*;
use crate::utils;

use super::bp::ByteProvider;
use super::cc::CharClass;
use super::tk::{Token, Tokenizer};

pub struct Parser<T: ByteProvider + Seek> {
    tkn: Tokenizer<T>
}

impl<T: ByteProvider + Seek> Parser<T> {
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
            [b'1'..=b'9', ..] => {
                self.tkn.unread(first);
                self.read_number_or_indirect() },
            [b'+' | b'-' | b'0' | b'.', ..] => {
                self.tkn.unread(first);
                self.read_number().map(Object::Number) },
            b"(" => self.read_lit_string(),
            b"<" => self.read_hex_string(),
            b"/" => self.read_name().map(Object::Name),
            b"[" => self.read_array(),
            b"<<" => self.read_dict(),
            _ => Err(Error::Parse("unexcepted token")),
        }
    }

    fn read_number(&mut self) -> Result<Number, Error> {
        Self::to_number_inner(&self.tkn.next()?)
            .map_err(|_| Error::Parse("malformed number"))
    }

    fn read_number_or_indirect(&mut self) -> Result<Object, Error> {
        let num = Self::to_number_inner(&self.tkn.next()?)
            .map_err(|_| Error::Parse("malformed number"))?;
        let Number::Int(num) = num else {
            return Ok(Object::Number(num))
        };
        assert!(num >= 0);
        let gen_tk = self.tkn.next()?;
        if matches!(&gen_tk[..], [b'0'] | [b'1'..b'9', ..]) {
            match utils::parse_num(&gen_tk) {
                Some(gen) => {
                    let r_tk = self.tkn.next()?;
                    if r_tk == b"R" {
                        return Ok(Object::Ref(ObjRef{num: num as u64, gen}));
                    } else {
                        self.tkn.unread(r_tk);
                        self.tkn.unread(gen_tk);
                    }
                },
                None => self.tkn.unread(gen_tk)
            }
        } else {
            self.tkn.unread(gen_tk)
        }
        Ok(Object::Number(Number::Int(num)))
    }

    fn to_number_inner(tok: &Token) -> Result<Number, Error> {
        if tok.contains(&b'e') || tok.contains(&b'E') {
            return Err(Error::Parse("malformed number"))
        }
        if tok.contains(&b'.') {
            Ok(Number::Real(utils::parse_num(tok).ok_or(Error::Parse("malformed number"))?))
        } else {
            Ok(Number::Int(utils::parse_num(tok).ok_or(Error::Parse("malformed number"))?))
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
        loop {
            let tk = self.tkn.next()?;
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
                _ => return Err(Error::Parse("malformed dictionary"))
            };
            let value = self.read_obj()?;
            dict.push((key, value));
        }
        Ok(Object::Dict(Dict(dict)))
    }

    pub fn entrypoint(&mut self) -> Result<u64, Error> {
        let bytes = self.tkn.bytes();
        let len = bytes.seek(std::io::SeekFrom::End(0))?;
        let buf_size = std::cmp::min(len, 1024);

        // Read last 1024 bytes
        bytes.seek(std::io::SeekFrom::End(-(buf_size as i64)))?;
        // FIXME: use read_buf_exact when stabilized
        let mut data = vec![0; buf_size as usize];
        bytes.read_exact(&mut data)?;

        // Find "startxref<EOL>number<EOL>"
        let sxref = data.windows(9)
            .rposition(|w| w == b"startxref")
            .ok_or(Error::Parse("startxref not found"))?;
        let mut cur = Cursor::new(&data[sxref..]);
        cur.skip_past_eol()?;
        let sxref = utils::parse_num(&cur.read_line_excl()?).ok_or(Error::Parse("malformed startxref"))?;
        Ok(sxref)
    }

    fn read_obj_indirect(&mut self) -> Result<(ObjRef, Object), Error> {
        let Ok(Number::Int(num)) = self.read_number() else { return Err(Error::Parse("unexpected token")) };
        let num = num.try_into().map_err(|_| Error::Parse("invalid object number"))?;
        let Ok(Number::Int(gen)) = self.read_number() else { return Err(Error::Parse("unexpected token")) };
        let gen = gen.try_into().map_err(|_| Error::Parse("invalid generation number"))?;
        let oref = ObjRef{num, gen};
        if self.tkn.next()? != b"obj" {
            return Err(Error::Parse("unexpected token"));
        }
        let obj = self.read_obj()?;
        match &self.tkn.next()?[..] {
            b"endobj" =>
                Ok((oref, obj)),
            b"stream" => {
                let Object::Dict(dict) = obj else {
                    return Err(Error::Parse("endobj not found"))
                };
                let bytes = self.tkn.bytes();
                match bytes.next_or_eof()? {
                    b'\n' => (),
                    b'\r' => {
                        if bytes.next_or_eof()? != b'\n' {
                            return Err(Error::Parse("stream keyword not followed by proper EOL"));
                        }
                    },
                    _ => return Err(Error::Parse("stream keyword not followed by proper EOL"))
                };
                let stm = Stream { dict, data: Data::Ref(bytes.stream_position()?) };
                Ok((oref, Object::Stream(stm)))
            },
            _ => Err(Error::Parse("endobj not found"))
        }
    }

    pub fn read_xref_at(&mut self, start: u64) -> Result<(XRefType, Box<dyn XRefRead + '_>), Error> {
        self.seek_to(start)?;
        let tk = self.tkn.next()?;
        if tk == b"xref" {
            self.tkn.bytes().skip_past_eol()?;
            Ok((XRefType::Table, Box::new(ReadXRefTable::new(self)?)))
        } else {
            self.tkn.unread(tk);
            let (oref, obj) = self.read_obj_indirect()?;
            Ok((XRefType::Stream(oref), Box::new(ReadXRefStream::new(self, obj)?)))
        }
    }

    pub fn read_obj_at(&mut self, start: u64, oref_exp: &ObjRef) -> Result<Object, Error> {
        self.seek_to(start)?;
        let (oref, obj) = self.read_obj_indirect()?;
        if oref != *oref_exp {
            return Err(Error::Parse("object number mismatch"));
        }
        Ok(obj)
    }

    pub fn read_raw(&mut self, start: u64) -> Result<impl Read + use<'_, T>, Error> {
        self.seek_to(start)?;
        Ok(self.tkn.bytes())
    }
}

impl<T: Into<String>> From<T> for Parser<Cursor<String>> {
    fn from(input: T) -> Self {
        Parser { tkn: Tokenizer::from(input) }
    }
}


pub trait XRefRead : Iterator<Item = Result<(u64, Record), Error>> {
    fn trailer(self: Box<Self>) -> Result<Dict, Error>;
}


struct ReadXRefTable<'a, T: ByteProvider + Seek> {
    parser: &'a mut Parser<T>,
    num: u64,
    ceil: u64
}

impl<'a, T: ByteProvider + Seek> ReadXRefTable<'a, T> {
    fn new(parser: &'a mut Parser<T>) -> Result<Self, Error> {
        let bytes = parser.tkn.bytes();
        let line = bytes.read_line_excl()?.trim_ascii_end().to_owned();
        let (start, size) = Self::read_section(&line).map_err(|_| Error::Parse("malformed xref table"))?;
        Ok(Self { parser, num: start, ceil: start + size })
    }

    fn read_line(&mut self) -> Result<Record, ()> {
        let bytes = self.parser.tkn.bytes();
        let mut line = [0u8; 20];
        bytes.read_exact(&mut line).map_err(|_| ())?;
        if line[10] != b' ' || line[16] != b' ' {
            return Err(());
        }
        let v = utils::parse_num(&line[0..10]).ok_or(())?;
        let gen = utils::parse_num(&line[11..16]).ok_or(())?;
        match line[17] {
            b'n' => Ok(Record::Used{gen, offset: v}),
            b'f' => Ok(Record::Free{gen, next: v}),
            _ => Err(())
        }
    }

    fn read_section(line: &[u8]) -> Result<(u64, u64), ()> {
        let index = line.iter().position(|c| *c == b' ').ok_or(())?;
        let start = utils::parse_num(&line[..index]).ok_or(())?;
        let size = utils::parse_num(&line[(index+1)..]).ok_or(())?;
        Ok((start, size))
    }
}

impl<T: ByteProvider + Seek> Iterator for ReadXRefTable<'_, T> {
    type Item = Result<(u64, Record), Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.num == self.ceil {
            let bytes = self.parser.tkn.bytes();
            let line = match bytes.read_line_excl() {
                Ok(line) if line == b"trailer" => return None,
                Ok(line) => line.trim_ascii_end().to_owned(),
                Err(err) => return Some(Err(err.into()))
            };
            (self.num, self.ceil) = match Self::read_section(&line) {
                Ok((start, size)) => (start, start + size),
                Err(()) => return Some(Err(Error::Parse("malformed xref table")))
            };
        }
        if self.num < self.ceil {
            let ret = match self.read_line() {
                Ok(record) => Some(Ok((self.num, record))),
                Err(()) => Some(Err(Error::Parse("malformed xref table")))
            };
            self.num += 1;
            ret
        } else {
            None
        }
    }
}

impl<T: ByteProvider + Seek> XRefRead for ReadXRefTable<'_, T> {
    fn trailer(mut self: Box<Self>) -> Result<Dict, Error> {
        while self.num < self.ceil {
            let bytes = self.parser.tkn.bytes();
            bytes.seek(std::io::SeekFrom::Current(20 * ((self.ceil - self.num) as i64)))?;
            let line = bytes.read_line_excl()?.trim_ascii_end().to_owned();
            if line == b"trailer" { break; }
            let (start, size) = Self::read_section(&line).map_err(|_| Error::Parse("malformed xref table"))?;
            self.num = start;
            self.ceil = start + size;
        }
        match self.parser.read_obj()? {
            Object::Dict(dict) => Ok(dict),
            _ => Err(Error::Parse("malformed trailer"))
        }
    }
}


struct ReadXRefStream<'a> {
    dict: Dict,
    reader: Box<dyn Read + 'a>,
    // FIXME: use iter_array_chunks when stabilized
    index_iter: <Vec<u64> as IntoIterator>::IntoIter,
    widths: [usize; 3],
    num: u64,
    ceil: u64
}

impl<'a> ReadXRefStream<'a> {
    fn new<T: ByteProvider + Seek>(parser: &'a mut Parser<T>, obj: Object) -> Result<Self, Error> {
        let Object::Stream(Stream{dict, data: Data::Ref(offset)}) = obj
            else { return Err(Error::Parse("malformed xref")) };
        if dict.lookup(b"Type") != &Object::new_name("XRef") {
            return Err(Error::Parse("malfomed xref stream (/Type)"))
        }
        let &Object::Number(Number::Int(size)) = dict.lookup(b"Size") else {
            return Err(Error::Parse("malfomed xref stream (/Size)"))
        };
        if size <= 0 {
            return Err(Error::Parse("malfomed xref stream (/Size)"))
        }
        let size = size as u64;
        let index = match dict.lookup(b"Index") {
            Object::Array(arr) =>
                arr.iter()
                    .map(|obj| match obj {
                        &Object::Number(Number::Int(num)) if num >= 0 => Ok(num as u64),
                        _ => Err(Error::Parse("malfomed xref stream (/Index)"))
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            Object::Null => vec![0, size],
            _ => return Err(Error::Parse("malfomed xref stream (/Index)"))
        };
        let mut iter = index.into_iter();
        let (start, size) = match (iter.next(), iter.next()) {
            (Some(start), Some(size)) => (start, size),
            _ => (0, 0)
        };

        let widths : [_; 3] = match dict.lookup(b"W") {
            Object::Array(arr) =>
                arr.iter()
                    .map(|obj| match obj {
                        &Object::Number(Number::Int(num)) if (0..8).contains(&num) => Ok(num as usize),
                        _ => Err(Error::Parse("malfomed xref stream (/W)"))
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            _ => return Err(Error::Parse("malfomed xref stream (/W)"))
        }.try_into().map_err(|_| Error::Parse("malfomed xref stream (/W)"))?;
        if widths[1] == 0 {
            return Err(Error::Parse("malfomed xref stream (/W)"))
        }

        let &Object::Number(Number::Int(len)) = dict.lookup(b"Length") else {
            return Err(Error::Parse("malfomed xref stream (/Length)"))
        };
        let raw_reader = parser.read_raw(offset)?.take(len as u64);
        let reader = crate::codecs::decode(raw_reader, dict.lookup(b"Filter"));
        Ok(Self {
            dict, reader,
            index_iter: iter, widths,
            num: start, ceil: start + size
        })
    }

    fn read(&mut self, width: usize) -> Result<u64, Error> {
        let mut dec_buf = [0; 8];
        self.reader.read_exact(&mut dec_buf[(8-width)..8])?;
        Ok(u64::from_be_bytes(dec_buf))
    }

    fn read_line(&mut self) -> Result<Record, Error> {
        let [w1, w2, w3] = self.widths;
        let tpe = if w1 > 0 { self.read(w1)? } else { 1 };
        let f2 = self.read(w2)?;
        let f3 = self.read(w3)?.try_into()
            .expect("Generation field larger than 16 bits.");
        Ok(match tpe {
            0 => Record::Free{gen: f3, next: f2},
            1 => Record::Used{gen: f3, offset: f2},
            2 => Record::Compr{num: f2, index: f3},
            _ => unimplemented!()
        })
    }
}

impl Iterator for ReadXRefStream<'_> {
    type Item = Result<(u64, Record), Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.num == self.ceil {
            let start = self.index_iter.next()?;
            let size = self.index_iter.next()?;
            self.num = start;
            self.ceil = start + size;
        }
        if self.num < self.ceil {
            let ret = match self.read_line() {
                Ok(record) => Some(Ok((self.num, record))),
                Err(err) => Some(Err(err))
            };
            self.num += 1;
            ret
        } else {
            None
        }
    }

    /*FIXME Check after end?
       if !deflater.fill_buf()?.is_empty() {
        return Err(Error::Parse("malfomed xref stream"));
    }*/
}

impl XRefRead for ReadXRefStream<'_> {
    fn trailer(self: Box<Self>) -> Result<Dict, Error> {
        Ok(self.dict)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_obj() {
        let mut parser = Parser::from("true false null 123 +17 -98 0 00987 34.5 -3.62 +123.6 4. -.002 0.0 009.87");
        assert_eq!(parser.read_obj().unwrap(), Object::Bool(true));
        assert_eq!(parser.read_obj().unwrap(), Object::Bool(false));
        assert_eq!(parser.read_obj().unwrap(), Object::Null);
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Int(123)));
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Int(17)));
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Int(-98)));
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Int(0)));
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Int(987)));
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Real(34.5)));
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Real(-3.62)));
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Real(123.6)));
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Real(4.)));
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Real(-0.002)));
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Real(0.)));
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Real(9.87)));

        let mut parser = Parser::from("9223372036854775807 9223372036854775808");
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Int(9223372036854775807)));
        assert!(parser.read_obj().is_err());

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

        let mut parser = Parser::from("<61\r\n6 2> <61%comment\n> <61%unterminated>");
        assert_eq!(parser.read_obj().unwrap(), Object::new_string("ab"));
        assert_eq!(parser.read_obj().unwrap(), Object::new_string("a"));
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
            (Name::from("Length"), Object::Ref(ObjRef{num: 8, gen: 0}))
        ])));

        let mut parser = Parser::from("1 2 3 R 4 R");
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Int(1)));
        assert_eq!(parser.read_obj().unwrap(), Object::Ref(ObjRef{num: 2, gen: 3}));
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Int(4)));
        assert!(parser.read_obj().is_err());

        let mut parser = Parser::from("0 0 R");
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Int(0)));
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Int(0)));
        assert!(parser.read_obj().is_err());

        let mut parser = Parser::from("01 0 R");
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Int(1)));
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Int(0)));
        assert!(parser.read_obj().is_err());

        let mut parser = Parser::from("1 01 R");
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Int(1)));
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Int(1)));
        assert!(parser.read_obj().is_err());

        let mut parser = Parser::from("1 +1 R");
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Int(1)));
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Int(1)));
        assert!(parser.read_obj().is_err());
    }
}
