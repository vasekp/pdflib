use std::io::{Cursor, Seek, Read, BufRead};

use crate::base::*;
use crate::base::types::*;
use crate::utils;

use super::bp::ByteProvider;
use super::op::ObjParser;

pub struct FileParser<T: BufRead + Seek> {
    op: ObjParser<T>
}

impl<T: BufRead + Seek> FileParser<T> {
    pub fn new(reader: T) -> Self {
        Self { op: ObjParser::new(reader) }
    }

    fn seek_to(&mut self, pos: Offset) -> std::io::Result<u64> {
        self.op.tkn.bytes().seek(std::io::SeekFrom::Start(pos))
    }

    pub fn read_xref_at(&mut self, start: Offset) -> Result<(XRefType, Box<dyn XRefRead + '_>), Error> {
        self.seek_to(start)?;
        let tk = self.op.tkn.next()?;
        if tk == b"xref" {
            self.op.tkn.bytes().skip_past_eol()?;
            Ok((XRefType::Table, Box::new(ReadXRefTable::new(self)?)))
        } else {
            self.op.tkn.unread(tk);
            let (oref, obj) = self.read_obj_indirect(&())?;
            Ok((XRefType::Stream(oref), Box::new(ReadXRefStream::new(self, obj)?)))
        }
    }

    pub fn read_obj_at(&mut self, start: Offset, locator: &(impl Locator + ?Sized)) -> Result<(ObjRef, Object), Error> {
        self.seek_to(start)?;
        self.read_obj_indirect(locator)
    }

    pub fn read_raw(&mut self, start: Offset) -> Result<impl Read + use<'_, T>, Error> {
        self.seek_to(start)?;
        Ok(self.op.tkn.bytes())
    }

    pub fn find_header(&mut self) -> Result<Header, Error> {
        const BUF_SIZE: usize = 1024;
        const HEADER_FIXED: [u8; 5] = *b"%PDF-";
        const HEADER_FIXED_LEN: usize = HEADER_FIXED.len();
        const HEADER_FULL_LEN: usize = HEADER_FIXED_LEN + 3;
        const OVERLAP: usize = HEADER_FULL_LEN - 1;

        let mut data = vec![0u8; HEADER_FULL_LEN];
        let mut from = 0;
        let mut to = data.len();
        use std::ops::ControlFlow;
        let try_find = |data: &[u8], from: usize| {
            data.windows(HEADER_FULL_LEN)
                .enumerate()
                .filter(|(_, w)| w[0..HEADER_FIXED_LEN] == HEADER_FIXED)
                .try_fold((), |(), (ix, w)| match &w[HEADER_FIXED_LEN..] {
                    [maj @ b'0'..=b'9', b'.', min @ b'0'..=b'9'] => {
                        let start = (from + ix).try_into().expect("Should fit into u64.");
                        let version = (maj - b'0', min - b'0');
                        ControlFlow::Break(Header { start, version })
                    },
                    _ => ControlFlow::Continue(())
                })
                .break_value()
        };

        let bytes = self.op.tkn.bytes();
        let file_len = bytes.seek(std::io::SeekFrom::End(0))?
            .try_into().expect("File length should fit into usize.");
        bytes.seek(std::io::SeekFrom::Start(0))?;

        bytes.read_exact(&mut data)?;
        if let Some(header) = try_find(&data, from) {
            return Ok(header);
        }

        while to < file_len {
            let data_len = data.len();
            data.copy_within((data_len - OVERLAP).., 0);
            from = to - OVERLAP;
            to = std::cmp::min(from + BUF_SIZE, file_len);
            data.resize(to - from, 0u8);
            bytes.read_exact(&mut data[OVERLAP..])?;
            if let Some(header) = try_find(&data, from) {
                return Ok(header);
            }
        }

        Err(Error::Parse("header not found"))
    }

    pub fn entrypoint(&mut self) -> Result<Offset, Error> {
        let bytes = self.op.tkn.bytes();
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

    fn read_obj_indirect(&mut self, locator: &(impl Locator + ?Sized)) -> Result<(ObjRef, Object), Error> {
        // TODO: check format of num and gen
        let Ok(Number::Int(num)) = self.op.read_number() else { return Err(Error::Parse("unexpected token")) };
        let num = num.try_into().map_err(|_| Error::Parse("invalid object number"))?;
        let Ok(Number::Int(gen)) = self.op.read_number() else { return Err(Error::Parse("unexpected token")) };
        let gen = gen.try_into().map_err(|_| Error::Parse("invalid generation number"))?;
        let oref = ObjRef{num, gen};
        if self.op.tkn.next()? != b"obj" {
            return Err(Error::Parse("unexpected token"));
        }
        let obj = self.op.read_obj()?;
        match &self.op.tkn.next()?[..] {
            b"endobj" =>
                Ok((oref, obj)),
            b"stream" => {
                let Object::Dict(dict) = obj else {
                    return Err(Error::Parse("endobj not found"))
                };
                let bytes = self.op.tkn.bytes();
                match bytes.next_or_eof()? {
                    b'\n' => (),
                    b'\r' => {
                        if bytes.next_or_eof()? != b'\n' {
                            return Err(Error::Parse("stream keyword not followed by proper EOL"));
                        }
                    },
                    _ => return Err(Error::Parse("stream keyword not followed by proper EOL"))
                };
                let offset = bytes.stream_position()?;
                let len = self.resolve(dict.lookup(b"Length"), locator)
                    .ok().as_ref().and_then(Object::num_value);
                let filters = match self.resolve(dict.lookup(b"Filter"), locator).ok() {
                    Some(Object::Name(name)) => vec![name],
                    Some(Object::Array(vec)) => vec.iter()
                        .map(|obj| match self.resolve(obj, locator).ok() {
                            Some(Object::Name(name)) => Some(name),
                            _ => None
                        })
                        .collect::<Option<Vec<_>>>()
                        .unwrap_or(vec![]),
                    _ => vec![]
                };
                let stm = Stream { dict, data: Data::Ref(IndirectData { offset, len, filters }) };
                Ok((oref, Object::Stream(stm)))
            },
            _ => Err(Error::Parse("endobj not found"))
        }
    }

    fn resolve(&mut self, obj: &Object, locator: &(impl Locator + ?Sized)) -> Result<Object, Error> {
        if let Object::Ref(objref) = obj {
            let Some(offset) = locator.locate_offset(objref) else {
                return Ok(Object::Null)
            };
            let (readref, obj) = self.read_obj_at(offset, locator)?;
            if &readref == objref {
                Ok(obj)
            } else {
                Err(Error::Parse("object number mismatch"))
            }
        } else {
            Ok(obj.clone())
        }
    }
}


pub trait XRefRead : Iterator<Item = Result<(ObjNum, Record), Error>> {
    fn trailer(self: Box<Self>) -> Result<Dict, Error>;
}


struct ReadXRefTable<'a, T: ByteProvider + Seek> {
    parser: &'a mut FileParser<T>,
    num: ObjNum,
    ceil: ObjNum
}

impl<'a, T: ByteProvider + Seek> ReadXRefTable<'a, T> {
    fn new(parser: &'a mut FileParser<T>) -> Result<Self, Error> {
        let bytes = parser.op.tkn.bytes();
        let line = bytes.read_line_excl()?.trim_ascii_end().to_owned();
        let (start, size) = Self::read_section(&line).map_err(|_| Error::Parse("malformed xref table"))?;
        Ok(Self { parser, num: start, ceil: start + size })
    }

    fn read_line(&mut self) -> Result<Record, ()> {
        let bytes = self.parser.op.tkn.bytes();
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

    fn read_section(line: &[u8]) -> Result<(ObjNum, ObjNum), ()> {
        let index = line.iter().position(|c| *c == b' ').ok_or(())?;
        let start = utils::parse_num(&line[..index]).ok_or(())?;
        let size = utils::parse_num(&line[(index+1)..]).ok_or(())?;
        Ok((start, size))
    }
}

impl<T: ByteProvider + Seek> Iterator for ReadXRefTable<'_, T> {
    type Item = Result<(ObjNum, Record), Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.num == self.ceil {
            let bytes = self.parser.op.tkn.bytes();
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
            let bytes = self.parser.op.tkn.bytes();
            let skip: i64 = (self.ceil - self.num).try_into().map_err(|_| Error::Parse("range too large"))?;
            bytes.seek(std::io::SeekFrom::Current(20 * skip))?;
            let line = bytes.read_line_excl()?.trim_ascii_end().to_owned();
            if line == b"trailer" { break; }
            let (start, size) = Self::read_section(&line).map_err(|_| Error::Parse("malformed xref table"))?;
            self.num = start;
            self.ceil = start + size;
        }
        match self.parser.op.read_obj()? {
            Object::Dict(dict) => Ok(dict),
            _ => Err(Error::Parse("malformed trailer"))
        }
    }
}


struct ReadXRefStream<'a> {
    dict: Dict,
    reader: Box<dyn Read + 'a>,
    // FIXME: use iter_array_chunks when stabilized
    index_iter: <Vec<ObjNum> as IntoIterator>::IntoIter,
    widths: [usize; 3],
    num: ObjNum,
    ceil: ObjNum
}

impl<'a> ReadXRefStream<'a> {
    fn new<T: ByteProvider + Seek>(parser: &'a mut FileParser<T>, obj: Object) -> Result<Self, Error> {
        let Object::Stream(Stream{dict, data: Data::Ref(ind_data)}) = obj
            else { return Err(Error::Parse("malformed xref")) };
        if dict.lookup(b"Type") != &Object::new_name("XRef") {
            return Err(Error::Parse("malfomed xref stream (/Type)"))
        }
        let size = dict.lookup(b"Size").num_value()
            .ok_or(Error::Parse("malfomed xref stream (/Size)"))?;
        let index = match dict.lookup(b"Index") {
            Object::Array(arr) =>
                arr.iter()
                    .map(|obj| obj.num_value().ok_or(Error::Parse("malfomed xref stream (/Index)")))
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

        let IndirectData{offset, len: Some(len), filters} = ind_data else {
            return Err(Error::Parse("malfomed xref stream (/Length)"));
        };
        let raw_reader = parser.read_raw(offset)?.take(len);
        let reader = crate::codecs::decode(raw_reader, &filters);
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
    type Item = Result<(ObjNum, Record), Error>;

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
    //use super::*;
    //TODO
}
