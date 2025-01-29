use std::io::{Cursor, Seek, Read, BufRead};
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::cell::RefCell;
use std::ops::DerefMut;

use crate::base::*;
use crate::base::types::*;
use crate::utils;

use super::bp::ByteProvider;
use super::op::ObjParser;
use super::tk::Tokenizer;

pub struct FileParser<T: BufRead + Seek> {
    reader: RefCell<T>,
    header: Result<Header, Error>,
}

pub enum Structural {
    Object(ObjRef, Object),
    XRefSec(XRef)
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
        Self { reader: RefCell::new(reader), header }
    }

    fn start(&self) -> Offset {
        match self.header {
            Ok(Header{ start, .. }) => start,
            _ => 0
        }
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

    pub fn entrypoint(&self) -> Result<Offset, Error> {
        let mut reader = self.reader.borrow_mut();
        let len = reader.seek(std::io::SeekFrom::End(0))?;
        let buf_size = std::cmp::min(len, 1024);

        // Read last 1024 bytes
        reader.seek(std::io::SeekFrom::End(-(buf_size as i64)))?;
        // FIXME: use read_buf_exact when stabilized
        let mut data = vec![0; buf_size as usize];
        reader.read_exact(&mut data)?;

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

    fn read_at(&self, pos: Offset) -> Result<Structural, Error> {
        let mut reader = self.reader.borrow_mut();
        reader.seek(std::io::SeekFrom::Start(pos + self.start()))?;
        let tk = reader.read_token_nonempty()?;
        if tk == b"xref" {
            reader.read_eol()?;
            let xref = self.read_xref_table(reader.deref_mut())?;
            return Ok(Structural::XRefSec(xref));
        }
        let num = utils::parse_int_strict(&tk)
            .ok_or(Error::Parse("invalid object number"))?;
        let tk = reader.read_token_nonempty()?;
        let gen = utils::parse_int_strict(&tk)
            .ok_or(Error::Parse("invalid generation number"))?;
        let oref = ObjRef{num, gen};
        if reader.read_token_nonempty()? != b"obj" {
            return Err(Error::Parse("unexpected token"));
        }
        let obj = ObjParser::read_obj(reader.deref_mut())?;
        match &reader.read_token_nonempty()?[..] {
            b"endobj" =>
                Ok(Structural::Object(oref, obj)),
            b"stream" => {
                let Object::Dict(dict) = obj else {
                    return Err(Error::Parse("endobj not found"))
                };
                match reader.next_or_eof()? {
                    b'\n' => (),
                    b'\r' => {
                        if reader.next_or_eof()? != b'\n' {
                            return Err(Error::Parse("stream keyword not followed by proper EOL"));
                        }
                    },
                    _ => return Err(Error::Parse("stream keyword not followed by proper EOL"))
                };
                let offset = reader.stream_position()?;
                let stm = Stream { dict, data: Data::Ref(offset) };
                Ok(Structural::Object(oref, Object::Stream(stm)))
            },
            _ => Err(Error::Parse("endobj not found"))
        }
    }

    pub fn read_obj_at(&self, pos: Offset) -> Result<(ObjRef, Object), Error> {
        match self.read_at(pos)? {
            Structural::Object(oref, obj) => Ok((oref, obj)),
            _ => Err(Error::Parse("expected object, found xref section"))
        }
    }

    pub fn read_xref_at(&mut self, pos: Offset) -> Result<XRef, Error> {
        match self.read_at(pos)? {
            Structural::XRefSec(xref) => Ok(xref),
            Structural::Object(oref, obj) => self.read_xref_stream(oref, obj)
        }
    }

    fn read_xref_table(&self, reader: &mut T) -> Result<XRef, Error> {
        let mut map = BTreeMap::new();
        let err = || Error::Parse("malformed xref table");
        loop {
            let line = reader.read_line_excl()?.trim_ascii_end().to_owned();
            if line == b"trailer" { break; }
            let index = line.iter().position(|c| *c == b' ').ok_or_else(err)?;
            let start = utils::parse_num::<u64>(&line[..index]).ok_or_else(err)?;
            let size = utils::parse_num::<u64>(&line[(index+1)..]).ok_or_else(err)?;
            let mut line = [0u8; 20];
            for num in start..(start+size) {
                reader.read_exact(&mut line)?;
                if line[10] != b' ' || line[16] != b' ' {
                    return Err(err());
                }
                let v = utils::parse_num::<u64>(&line[0..10]).ok_or_else(err)?;
                let gen = utils::parse_num::<u16>(&line[11..16]).ok_or_else(err)?;
                let rec = match line[17] {
                    b'n' => Record::Used{gen, offset: v},
                    b'f' => Record::Free{gen, next: v},
                    _ => return Err(err())
                };
                match map.entry(num) {
                    Entry::Vacant(entry) => { entry.insert(rec); },
                    Entry::Occupied(_) => log::warn!("Duplicate object number {num} in xref table")
                };
            }
        }
        let trailer = match ObjParser::read_obj(reader)? {
            Object::Dict(dict) => dict,
            _ => return Err(Error::Parse("malformed trailer"))
        };
        let size = trailer.lookup(b"Size")
            .num_value()
            .ok_or(Error::Parse("malformed trailer (missing /Size)"))?;
        Ok(XRef { tpe: XRefType::Table, map, dict: trailer, size })
    }

    fn read_xref_stream(&mut self, oref: ObjRef, obj: Object) -> Result<XRef, Error> {
        let mut reader = self.reader.borrow_mut();
        let Object::Stream(Stream{dict, data: Data::Ref(offset)}) = obj else {
            return Err(Error::Parse("malfomed xref"))
        };
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

        let [w1, w2, w3] = match dict.lookup(b"W") {
            Object::Array(arr) =>
                arr.iter()
                    .map(|obj| match obj {
                        &Object::Number(Number::Int(num)) if (0..=8).contains(&num) => Ok(num as usize),
                        _ => Err(Error::Parse("malfomed xref stream (/W)"))
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            _ => return Err(Error::Parse("malfomed xref stream (/W)"))
        }.try_into().map_err(|_| Error::Parse("malfomed xref stream (/W)"))?;
        if w2 == 0 {
            return Err(Error::Parse("malfomed xref stream (/W)"))
        }

        assert_eq!(reader.stream_position()?, offset);
        let len = dict.lookup(b"Length")
            .num_value()
            .ok_or(Error::Parse("malfomed xref stream (/Length)"))?;
        let filters = match dict.lookup(b"Filter") {
            Object::Name(name) => vec![name.to_owned()],
            Object::Array(vec) => vec.iter()
                .map(|obj| match obj {
                    Object::Name(name) => Ok(name.to_owned()),
                    _ => Err(Error::Parse("malformed xref stream (/Filter)"))
                })
                .collect::<Result<Vec<_>, _>>()?,
            _ => vec![]
        };
        let codec_in = reader.deref_mut().take(len);
        let mut codec_out = crate::codecs::decode(codec_in, &filters);
        let mut read = |w| -> Result<u64, Error> {
            let mut dec_buf = [0; 8];
            codec_out.read_exact(&mut dec_buf[(8-w)..8])?;
            Ok(u64::from_be_bytes(dec_buf))
        };

        let mut map = BTreeMap::new();
        // FIXME: use array_chunks() when stabilized
        for ch in index.chunks_exact(2) {
            let &[start, len] = ch else { unreachable!() };
            for num in start..(start + len) {
                let tpe = if w1 > 0 { read(w1)? } else { 1 };
                let f2 = read(w2)?;
                let f3 = read(w3)?.try_into().expect("Generation field larger than 16 bits.");
                let rec = match tpe {
                    0 => Record::Free{gen: f3, next: f2},
                    1 => Record::Used{gen: f3, offset: f2},
                    2 => Record::Compr{num: f2, index: f3},
                    _ => unimplemented!("xref type {tpe}")
                };
                match map.entry(num) {
                    Entry::Vacant(entry) => { entry.insert(rec); },
                    Entry::Occupied(_) => log::warn!("Duplicate object number {num} in xref stream")
                };
            }
        }
        if !codec_out.fill_buf()?.is_empty() {
            return Err(Error::Parse("malfomed xref stream"));
        }
        Ok(XRef { tpe: XRefType::Stream(oref), map, dict, size })
    }
}


#[cfg(test)]
mod tests {
    //use super::*;
    //TODO
}
