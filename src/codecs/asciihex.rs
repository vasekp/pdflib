use std::io::*;
use crate::parser::tk::Tokenizer;
use crate::utils;

pub fn decode<R: BufRead>(input: R) -> BufReader<AsciiHexDecoder<R>> {
    BufReader::new(AsciiHexDecoder::new(input))
}

pub struct AsciiHexDecoder<R: BufRead> {
    reader: R,
    rem: Cursor<Vec<u8>>,
    done: bool
}

impl<R: BufRead> AsciiHexDecoder<R> {
    fn new(input: R) -> Self {
        AsciiHexDecoder {
            reader: input,
            rem: Default::default(),
            done: false
        }
    }

    fn next_in(&mut self) -> std::io::Result<Option<u8>> {
        if self.done { return Ok(None); }
        let mut buf = [0];
        if self.rem.read(&mut buf)? == 0 {
            self.rem = match self.reader.read_token_nonempty() {
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

    fn next_out(&mut self) -> std::io::Result<Option<u8>> {
        let msd = match self.next_in()? {
            Some(msd) => utils::hex_value(msd).ok_or(Error::from(ErrorKind::InvalidData))?,
            None => return Ok(None),
        };
        let lsd = utils::hex_value(self.next_in()?.unwrap_or(b'0'))
            .ok_or(Error::from(ErrorKind::InvalidData))?;
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
