use std::io::{Cursor, Seek, Read, BufRead};

use crate::base::*;
use crate::base::types::*;
use crate::utils;

use super::bp::ByteProvider;
use super::op::ObjParser;
use super::tk::{Tokenizer, Token};

pub struct FileParser<T: BufRead + Seek> {
    reader: T,
    header: Result<Header, Error>,
}

impl<T: BufRead + Seek> FileParser<T> {
    pub fn new(mut reader: T) -> Self {
        let header = Self::find_header(&mut reader);
        match &header {
            Ok(Header { start, version }) => {
                log::info!("PDF version {}.{}", version.0, version.1);
                if *start != 0 {
                    log::info!("Offset start @ {start}");
                }
            },
            Err(err) => log::warn!("{}", err)
        }
        Self { reader, header }
    }

    fn seek_to(&mut self, pos: Offset) -> std::io::Result<u64> {
        self.reader.seek(std::io::SeekFrom::Start(pos))
    }

    fn start(&self) -> Offset {
        match self.header {
            Ok(Header{ start, .. }) => start,
            _ => 0
        }
    }

    pub fn read_xref_at(&mut self, start: Offset) -> Result<(XRefType, Box<dyn XRefRead + '_>), Error> {
        self.seek_to(start + self.start())?;
        let tk = self.reader.read_token_nonempty()?;
        if tk == b"xref" {
            self.reader.read_eol()?;
            Ok((XRefType::Table, Box::new(ReadXRefTable::new(&mut self.reader)?)))
        } else {
            let (oref, obj) = self.read_obj_indirect(Some(tk), &())?;
            Ok((XRefType::Stream(oref), Box::new(ReadXRefStream::new(&mut self.reader, obj)?)))
        }
    }

    pub fn read_obj_at(&mut self, start: Offset, locator: &(impl Locator + ?Sized)) -> Result<(ObjRef, Object), Error> {
        self.seek_to(start + self.start())?;
        self.read_obj_indirect(None, locator)
    }

    /*pub fn read_raw(&mut self, start: Offset) -> Result<impl BufRead + use<'_, T>, Error> {
        self.seek_to(start + self.start())?;
        Ok(&mut self.reader)
    }*/

    fn find_header(reader: &mut T) -> Result<Header, Error> {
        const BUF_SIZE: usize = 1024;
        const HEADER_FIXED: &[u8] = b"%PDF-";
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
                .filter(|(_, w)| w[0..HEADER_FIXED_LEN] == *HEADER_FIXED)
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

        let file_len = reader.seek(std::io::SeekFrom::End(0))?
            .try_into().expect("File length should fit into usize.");
        reader.seek(std::io::SeekFrom::Start(0))?;

        reader.read_exact(&mut data)?;
        if let Some(header) = try_find(&data, from) {
            return Ok(header);
        }

        while to < file_len {
            let data_len = data.len();
            data.copy_within((data_len - OVERLAP).., 0);
            from = to - OVERLAP;
            to = std::cmp::min(from + BUF_SIZE, file_len);
            data.resize(to - from, 0u8);
            reader.read_exact(&mut data[OVERLAP..])?;
            if let Some(header) = try_find(&data, from) {
                return Ok(header);
            }
        }

        Err(Error::Parse("header not found"))
    }

    pub fn entrypoint(&mut self) -> Result<Offset, Error> {
        let len = self.reader.seek(std::io::SeekFrom::End(0))?;
        let buf_size = std::cmp::min(len, 1024);

        // Read last 1024 bytes
        self.reader.seek(std::io::SeekFrom::End(-(buf_size as i64)))?;
        // FIXME: use read_buf_exact when stabilized
        let mut data = vec![0; buf_size as usize];
        self.reader.read_exact(&mut data)?;

        // Find "startxref<EOL>number<EOL>"
        const SXREF: &[u8] = b"startxref";
        let sxref = data.windows(SXREF.len())
            .rposition(|w| w == b"startxref")
            .ok_or(Error::Parse("startxref not found"))?;
        let mut cur = Cursor::new(&data[(sxref+SXREF.len())..]);
        cur.read_eol()?;
        let sxref = utils::parse_num(&cur.read_line_excl()?).ok_or(Error::Parse("malformed startxref"))?;
        Ok(sxref)
    }

    fn read_obj_indirect(&mut self, tk: Option<Token>, locator: &(impl Locator + ?Sized)) -> Result<(ObjRef, Object), Error> {
        let tk = match tk {
            Some(tk) => tk,
            None => self.reader.read_token_nonempty()?
        };
        let num = utils::parse_int_strict(&tk)
            .ok_or(Error::Parse("invalid object number"))?;
        let tk = self.reader.read_token_nonempty()?;
        let gen = utils::parse_int_strict(&tk)
            .ok_or(Error::Parse("invalid generation number"))?;
        let oref = ObjRef{num, gen};
        if self.reader.read_token_nonempty()? != b"obj" {
            return Err(Error::Parse("unexpected token"));
        }
        let obj = ObjParser::new(&mut self.reader).read_obj()?;
        match &self.reader.read_token_nonempty()?[..] {
            b"endobj" =>
                Ok((oref, obj)),
            b"stream" => {
                let Object::Dict(dict) = obj else {
                    return Err(Error::Parse("endobj not found"))
                };
                match self.reader.next_or_eof()? {
                    b'\n' => (),
                    b'\r' => {
                        if self.reader.next_or_eof()? != b'\n' {
                            return Err(Error::Parse("stream keyword not followed by proper EOL"));
                        }
                    },
                    _ => return Err(Error::Parse("stream keyword not followed by proper EOL"))
                };
                let offset = self.reader.stream_position()?;
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


struct ReadXRefTable<'a, T: BufRead + Seek> {
    reader: &'a mut T,
    num: ObjNum,
    ceil: ObjNum
}

impl<'a, T: BufRead + Seek> ReadXRefTable<'a, T> {
    fn new(reader: &'a mut T) -> Result<Self, Error> {
        let line = reader.read_line_excl()?.trim_ascii_end().to_owned();
        let (start, size) = Self::read_section(&line).map_err(|_| Error::Parse("malformed xref table"))?;
        Ok(Self { reader, num: start, ceil: start + size })
    }

    fn read_line(&mut self) -> Result<Record, ()> {
        let mut line = [0u8; 20];
        self.reader.read_exact(&mut line).map_err(|_| ())?;
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

impl<T: BufRead + Seek> Iterator for ReadXRefTable<'_, T> {
    type Item = Result<(ObjNum, Record), Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.num == self.ceil {
            let line = match self.reader.read_line_excl() {
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

impl<T: BufRead + Seek> XRefRead for ReadXRefTable<'_, T> {
    fn trailer(mut self: Box<Self>) -> Result<Dict, Error> {
        while self.num < self.ceil {
            let skip: i64 = (self.ceil - self.num).try_into().map_err(|_| Error::Parse("range too large"))?;
            self.reader.seek(std::io::SeekFrom::Current(20 * skip))?;
            let line = self.reader.read_line_excl()?.trim_ascii_end().to_owned();
            if line == b"trailer" { break; }
            let (start, size) = Self::read_section(&line).map_err(|_| Error::Parse("malformed xref table"))?;
            self.num = start;
            self.ceil = start + size;
        }
        match ObjParser::new(&mut self.reader).read_obj()? {
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
    fn new<T: BufRead + Seek>(reader: &'a mut T, obj: Object) -> Result<Self, Error> {
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
        reader.seek(std::io::SeekFrom::Start(offset))?;
        let codec_in = reader.take(len);
        let codec_out = crate::codecs::decode(codec_in, &filters);
        Ok(Self {
            dict, reader: codec_out,
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
            _ => unimplemented!("xref type {tpe}")
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
