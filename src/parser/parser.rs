use std::io::Cursor;

use crate::base::*;

use super::bp::ByteProvider;
use super::cc::CharClass;
use super::tk::{Token, Tokenizer};

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
                    return Ok(Object::Ref(ObjRef{num: num as u64, gen: gen as u16}));
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

    pub fn entrypoint(&mut self) -> Result<u64, Error> {
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
        let sxref = Self::parse::<u64>(&cur.read_line_excl()?)
            .map_err(|_| Error::Parse("malformed startxref"))?;
        Ok(sxref)
    }

    fn read_obj_indirect(&mut self) -> Result<(ObjRef, Object), Error> {
        let Number::Int(num) = self.read_number()? else { return Err(Error::Parse("unexpected token")) };
        let num = num.try_into().map_err(|_| Error::Parse("invalid object number"))?;
        let Number::Int(gen) = self.read_number()? else { return Err(Error::Parse("unexpected token")) };
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

    fn read_xref_table(&mut self) -> Result<XRef, Error> {
        let bytes = self.tkn.bytes();
        bytes.skip_past_eol()?;
        let mut table = std::collections::BTreeMap::new();
        let err = || Error::Parse("malformed xref table");
        loop {
            let line = bytes.read_line_excl()?.trim_ascii_end().to_owned();
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
        Ok(XRef{table, trailer, tpe: XRefType::Table})
    }

    fn read_xref_stream(&mut self) -> Result<XRef, Error> {
        let (oref, obj) = self.read_obj_indirect()?;
        let Object::Stream(Stream{dict, data: Data::Ref(offset)}) = obj else {
            return Err(Error::Parse("malfomed xref"))
        };
        if dict.lookup(b"Type") != Some(&Object::new_name("XRef")) {
            return Err(Error::Parse("malfomed xref stream (/Type)"))
        }
        let Some(&Object::Number(Number::Int(size))) = dict.lookup(b"Size") else {
            return Err(Error::Parse("malfomed xref stream (/Size)"))
        };
        if size <= 0 {
            return Err(Error::Parse("malfomed xref stream (/Size)"))
        }
        let size = size as u64;
        let index = match dict.lookup(b"Index") {
            Some(Object::Array(arr)) =>
                arr.iter()
                    .map(|obj| match obj {
                        &Object::Number(Number::Int(num)) if num >= 0 => Ok(num as u64),
                        _ => Err(Error::Parse("malfomed xref stream (/Index)"))
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            None => vec![0, size],
            _ => return Err(Error::Parse("malfomed xref stream (/Index)"))
        };
        let w = match dict.lookup(b"W") {
            Some(Object::Array(arr)) =>
                arr.iter()
                    .map(|obj| match obj {
                        &Object::Number(Number::Int(num)) if num >= 0 && num < 8 => Ok(num as usize),
                        _ => Err(Error::Parse("malfomed xref stream (/W)"))
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            _ => return Err(Error::Parse("malfomed xref stream (/W)"))
        };
        let [w1, w2, w3] = w[..] else {
            return Err(Error::Parse("malfomed xref stream (/W)"))
        };
        if w2 == 0 {
            return Err(Error::Parse("malfomed xref stream (/W)"))
        }
        let Some(&Object::Number(Number::Int(len))) = dict.lookup(b"Length") else {
            return Err(Error::Parse("malfomed xref stream (/Length)"))
        };
        let mut table = std::collections::BTreeMap::new();
        assert_eq!(dict.lookup(b"Filter"), Some(&Object::new_name("FlateDecode"))); // TODO
        let data_raw = self.read_stream_data(offset, Some(len))?;
        use flate2::bufread::ZlibDecoder;
        use std::io::{Read, BufRead, BufReader};
        let mut deflater = BufReader::new(ZlibDecoder::new(&data_raw[..]));
        let mut read = |w| -> Result<u64, Error> {
            let mut dec_buf = [0; 8];
            deflater.read_exact(&mut dec_buf[(8-w)..8])?;
            Ok(u64::from_be_bytes(dec_buf))
        };
        // TODO: use array_chunks() when stabilized
        for ch in index.chunks_exact(2) {
            let &[start, len] = ch else { unreachable!() };
            for num in start..(start + len) {
                let tpe = if w1 > 0 { read(w1)? } else { 1 };
                let f2 = read(w2)?;
                let f3 = read(w3)?.try_into().expect("Generation field larger than 16 bits.");
                match tpe {
                    0 => table.insert(num, Record::Free{gen: f3, next: f2}),
                    1 => table.insert(num, Record::Used{gen: f3, offset: f2}),
                    2 => table.insert(num, Record::Compr{num: f2, index: f3}),
                    _ => unimplemented!()
                };
            }
        }
        if deflater.fill_buf()?.len() != 0 {
            return Err(Error::Parse("malfomed xref stream"));
        }
        Ok(XRef{table, trailer: dict, tpe: XRefType::Stream(oref)})
    }

    fn read_xref_inner(&mut self) -> Result<XRef, Error> {
        let tk = self.tkn.next()?;
        if tk == b"xref" {
            Ok(self.read_xref_table()?)
        } else {
            self.tkn.unread(tk);
            Ok(self.read_xref_stream()?)
        }
    }

    pub fn read_xref_at(&mut self, start: u64) -> Result<XRef, Error> {
        self.seek_to(start)?;
        self.read_xref_inner()
    }

    pub fn read_obj_at(&mut self, start: u64, oref_exp: &ObjRef) -> Result<Object, Error> {
        self.seek_to(start)?;
        let (oref, obj) = self.read_obj_indirect()?;
        if oref != *oref_exp {
            return Err(Error::Parse("object number mismatch"));
        }
        Ok(obj)
    }

    pub fn read_stream_data(&mut self, start: u64, len: Option<i64>) -> Result<Vec<u8>, Error> {
        self.seek_to(start)?;
        let data = match len {
            Some(len) => {
                if len <= 0 { return Err(Error::Parse("invalid length")); }
                let mut data = vec![0; len as usize];
                self.tkn.bytes().read_exact(&mut data)?;
                data
            },
            None => {
                let mut data = Vec::new();
                let bytes = self.tkn.bytes();
                loop {
                    let off = bytes.stream_position()?;
                    let line = bytes.read_line_incl()?;
                    if let Some(pos) = line.windows(9).position(|w| w == b"endstream") {
                        data.extend_from_slice(&line[0..pos]);
                        bytes.seek(std::io::SeekFrom::Start(off + (pos as u64)))?;
                        break;
                    } else {
                        data.extend_from_slice(&line[..]);
                    }
                }
                data
            }
        };
        if self.tkn.next()? != b"endstream" {
            return Err(Error::Parse("endstream not found"));
        }
        if self.tkn.next()? != b"endobj" {
            return Err(Error::Parse("endobj missing"));
        }
        Ok(data)
    }

    pub fn find_obj(&mut self, oref: &ObjRef, xref: &XRef) -> Result<Object, Error> {
        // TODO: /Size
        let Some(rec) = xref.table.get(&oref.num) else {
            return Ok(Object::Null);
        };
        match *rec {
            Record::Used{gen, offset} => {
                if gen != oref.gen {
                    Ok(Object::Null)
                } else {
                    self.read_obj_at(offset, oref)
                }
            },
            Record::Compr{..} => unimplemented!(),
            _ => Ok(Object::Null)
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
            (Name::from("Length"), Object::Ref(ObjRef{num: 8, gen: 0}))
        ])));

        let mut parser = Parser::from("1 2 3 R 4 R");
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Int(1)));
        assert_eq!(parser.read_obj().unwrap(), Object::Ref(ObjRef{num: 2, gen: 3}));
        assert_eq!(parser.read_obj().unwrap(), Object::Number(Number::Int(4)));
        assert!(parser.read_obj().is_err());
    }
}
