use std::io::{Cursor, Seek, Read, BufRead};
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::cell::RefCell;
use std::ops::DerefMut;

use crate::base::*;
use crate::base::types::*;
use crate::utils;
use crate::codecs;

use super::bp::ByteProvider;
use super::op::ObjParser;
use super::tk::Tokenizer;

/// The main interface to a file-level PDF parsing.
pub struct FileParser<T: BufRead + Seek> {
    reader: RefCell<T>,
    header: Result<Header, Error>,
}

pub enum Structural {
    Object(ObjRef, Object),
    XRefSec(XRef)
}

impl<T: BufRead + Seek> FileParser<T> {
    /// Creates a `FileParser` instance with the provided `BufRead`.
    ///
    /// Locates the PDF header, determining the PDF version and its byte offset within the stream.
    /// This information, along with the possible errors) is later available through a call to 
    /// [`FileParser::header()`].
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

    /// Opens a raw data reader starting at the specified file offset (relative to `%PDF`).
    ///
    /// Note that this is a mutable borrow of an internal `RefCell`, so in order to prevent runtime 
    /// borrow checking failures, you may need to manually `drop()` the instance prior to calling 
    /// any other methods of this `FileParser`.
    ///
    /// Also note that no length limit or stop condition is imposed, so this instance can be used 
    /// to read all the way to the end of the input. Use [`std::io::Read::take()`] to limit the 
    /// number of bytes read.
    pub fn read_raw(&self, pos: Offset) -> Result<impl std::io::BufRead + use<'_, T>, Error> {
        let mut reader = self.reader.borrow_mut();
        reader.seek(std::io::SeekFrom::Start(pos))?;
        Ok(StreamReader(reader))
    }

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

    /// Returns a reference to the `Result` of locating the PDF file header (during the call to 
    /// [`FileParser::new()`]).
    pub fn header(&self) -> &Result<Header, Error> {
        &self.header
    }

    /// Tries to locate the cross-reference entry point (`startxref`).
    ///
    /// The last 1024 bytes of the byte stream are inspected.
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
        let tk = reader.read_token()?;
        if tk == b"xref" {
            reader.read_eol()?;
            let xref = self.read_xref_table(&mut *reader)?;
            return Ok(Structural::XRefSec(xref));
        }
        let num = utils::parse_int_strict(&tk)
            .ok_or(Error::Parse("invalid object number"))?;
        let tk = reader.read_token()?;
        let gen = utils::parse_int_strict(&tk)
            .ok_or(Error::Parse("invalid generation number"))?;
        let oref = ObjRef{num, gen};
        if reader.read_token()? != b"obj" {
            return Err(Error::Parse("unexpected token"));
        }
        let obj = ObjParser::read_obj(&mut *reader)?;
        match &reader.read_token()?[..] {
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

    /// Attempts to read an indirect object at the specified location (relative to `%PDF`).
    pub fn read_obj_at(&self, pos: Offset) -> Result<(ObjRef, Object), Error> {
        match self.read_at(pos)? {
            Structural::Object(oref, obj) => Ok((oref, obj)),
            _ => Err(Error::Parse("expected object, found xref section"))
        }
    }

    /// Attempts to read a cross-reference table section or a cross-reference stream object at the 
    /// specified location (relative to `%PDF`).
    pub fn read_xref_at(&self, pos: Offset) -> Result<XRef, Error> {
        match self.read_at(pos)? {
            Structural::XRefSec(xref) => Ok(xref),
            Structural::Object(oref, obj) => self.read_xref_stream(oref, obj)
        }
    }

    fn read_xref_table(&self, reader: &mut T) -> Result<XRef, Error> {
        let mut map = BTreeMap::new();
        let err = || Error::Parse("malformed xref table");
        loop {
            let tk = reader.read_token()?;
            if tk == b"trailer" { break; }
            let start = utils::parse_num::<u64>(&tk).ok_or_else(err)?;
            let size = utils::parse_num::<u64>(&reader.read_token()?).ok_or_else(err)?;
            reader.skip_ws()?;
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

    fn read_xref_stream(&self, oref: ObjRef, obj: Object) -> Result<XRef, Error> {
        let mut reader = self.reader.borrow_mut();
        let Object::Stream(Stream{dict, data: Data::Ref(offset)}) = obj else {
            return Err(Error::Parse("malfomed xref"))
        };
        if dict.lookup(b"Type") != &Object::new_name(b"XRef") {
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
        let filters = codecs::to_filters(dict.lookup(b"Filter"))?;
        let params = match dict.lookup(b"DecodeParms") {
            Object::Dict(dict) => Some(dict),
            &Object::Null => None,
            _ => return Err(Error::Parse("malformed xref stream (/DecodeParms)"))
        };
        let codec_in = reader.deref_mut().take(len);
        let mut codec_out = codecs::decode(codec_in, &filters, params);
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
                    2 => Record::Compr{num_within: f2, index: f3},
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


struct StreamReader<'a, T: BufRead>(std::cell::RefMut<'a, T>);

impl<T: BufRead> std::io::Read for StreamReader<'_, T> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl<T: BufRead> BufRead for StreamReader<'_, T> {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        self.0.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        self.0.consume(amt)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::io::*;
    use std::fs::*;

    #[test]
    fn test_header_entrypoint() {
        let fp = FileParser::new(BufReader::new(File::open("src/tests/basic.pdf").unwrap()));
        let header = fp.header().as_ref().unwrap();
        assert_eq!(header.start, 0);
        assert_eq!(header.version, (1, 4));
        let entry = fp.entrypoint().unwrap();
        assert_eq!(entry, 1036);
        let mut r = fp.read_raw(entry).unwrap().take(4);
        let mut s = Vec::new();
        r.read_to_end(&mut s).unwrap();
        assert_eq!(s, b"xref");

        let fp = FileParser::new(BufReader::new(File::open("src/tests/offset.pdf").unwrap()));
        let entry = fp.entrypoint().unwrap();
        assert_eq!(entry, 4248);
        let offset = fp.header.as_ref().unwrap().start;
        assert_eq!(offset, 656);
        let start = entry + offset;
        let mut r = fp.read_raw(start).unwrap().take(4);
        let mut s = Vec::new();
        r.read_to_end(&mut s).unwrap();
        assert_eq!(s, b"xref");
    }

    #[test]
    fn test_read_xref() {
        let fp = FileParser::new(BufReader::new(File::open("src/tests/hybrid.pdf").unwrap()));
        let xref = fp.read_xref_at(912).unwrap();
        assert!(matches!(xref.tpe, XRefType::Table));
        assert_eq!(xref.dict.lookup(b"XRefStm").num_value(), Some(759));
        assert_eq!(xref.dict.lookup(b"Prev").num_value(), Some(417));

        let xref = fp.read_xref_at(759).unwrap();
        assert!(matches!(xref.tpe, XRefType::Stream(ObjRef { num: 6, gen: 0 })));
        assert_eq!(xref.dict.lookup(b"Type"), &Object::new_name(b"XRef"));

        let xref = fp.read_xref_at(417).unwrap();
        assert!(matches!(xref.tpe, XRefType::Table));
        assert_eq!(xref.dict.lookup(b"Size").num_value(), Some(6));

        // non-xref object
        assert!(fp.read_xref_at(251).is_err());
        assert!(fp.read_obj_at(251).is_ok());

        // no xref
        assert!(fp.read_xref_at(0).is_err());

        // This file has an empty line between xref and trailer.
        // Also, I deliberately point to a comment line preceding xref.
        let fp = FileParser::new(BufReader::new(File::open("src/tests/increment.pdf").unwrap()));
        let xref = fp.read_xref_at(5230).unwrap();
        assert!(matches!(xref.tpe, XRefType::Table));
    }

    #[test]
    fn test_read_obj_at() {
        let fp = FileParser::new(BufReader::new(File::open("src/tests/basic.pdf").unwrap()));

        let (oref, obj) = fp.read_obj_at(15).unwrap();
        assert_eq!(oref, ObjRef { num: 4, gen: 0 });
        let Object::Stream(Stream { data, .. }) = obj else { panic!() };
        assert_eq!(data, Data::Ref(74));

        // xref instead of object
        assert!(fp.read_obj_at(1036).is_err());
        assert!(fp.read_xref_at(1036).is_ok());
    }
}
