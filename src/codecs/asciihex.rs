use std::io::*;
use crate::parser::tk::Tokenizer;

pub fn decode<R: Read>(input: R) -> AsciiHexDecoder<BufReader<R>> {
    AsciiHexDecoder::new(input)
}

pub struct AsciiHexDecoder<R: BufRead> {
    tkn: Tokenizer<R>,
    rem: Cursor<Vec<u8>>,
    done: bool
}

impl<R: Read> AsciiHexDecoder<BufReader<R>> {
    fn new(input: R) -> Self {
        AsciiHexDecoder {
            tkn: Tokenizer::new(BufReader::new(input)),
            rem: Default::default(),
            done: false
        }
    }
}

impl<R: BufRead> AsciiHexDecoder<R> {
    fn next_in(&mut self) -> std::io::Result<Option<u8>> {
        if self.done { return Ok(None); }
        let mut buf = [0];
        if self.rem.read(&mut buf)? == 0 {
            self.rem = match self.tkn.next() {
                Ok(tk) => Cursor::new(tk),
                Err(err) if err.kind() == ErrorKind::UnexpectedEof
                    => { self.done = true; return Ok(None); },
                Err(err) => return Err(err)
            };
            assert!(self.rem.read(&mut buf)? == 1);
        }
        match buf[0] {
            b'>' => { self.done = true; Ok(None) },
            c => Ok(Some(c))
        }
    }

    fn hex_value(c: u8) -> std::io::Result<u8> {
        match c {
            b'0'..=b'9' => Ok(c - b'0'),
            b'a'..=b'f' => Ok(c - b'a' + 10),
            b'A'..=b'F' => Ok(c - b'A' + 10),
            _ => Err(Error::from(ErrorKind::InvalidData))
        }
    }

    fn next_out(&mut self) -> std::io::Result<Option<u8>> {
        let msd = match self.next_in()? {
            Some(msd) => Self::hex_value(msd)?,
            None => return Ok(None),
        };
        let lsd = Self::hex_value(self.next_in()?.unwrap_or(b'0'))?;
        Ok(Some((msd << 4) | lsd))
    }
}

impl<R: BufRead> Read for AsciiHexDecoder<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut bytes = 0;
        for b in buf {
            if let Some(c) = self.next_out()? {
                *b = c;
                bytes += 1;
            } else {
                break;
            }
        }
        Ok(bytes)
    }
}
