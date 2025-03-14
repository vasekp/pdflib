use std::io::*;
use crate::parser::cc::CharClass;

pub fn decode<R: BufRead>(input: R) -> BufReader<Ascii85Decoder<R>> {
    BufReader::new(Ascii85Decoder::new(input))
}

pub struct Ascii85Decoder<R: BufRead> {
    reader: R,
    buf: Vec<u8>,
    index: usize,
}

impl<R: BufRead> Ascii85Decoder<R> {
    fn new(input: R) -> Self {
        Ascii85Decoder {
            reader: input,
            buf: Vec::with_capacity(4),
            index: 0
        }
    }

    fn next_in(&mut self) -> std::io::Result<Option<u8>> {
        loop {
            match self.reader.fill_buf() {
                Ok([]) => return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof)),
                Ok([b'~', ..]) => return Ok(None),
                Ok(&[c, ..]) => {
                    self.reader.consume(1);
                    if CharClass::of(c) != CharClass::Space {
                        return Ok(Some(c));
                    }
                },
                Err(err) if err.kind() != std::io::ErrorKind::Interrupted => {
                    return Err(err);
                },
                _ => continue,
            }
        }
    }
}

impl<R: BufRead> BufRead for Ascii85Decoder<R> {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        if self.index < self.buf.len() {
            return Ok(&self.buf[self.index..]);
        }
        match self.next_in()? {
            None => Ok(&[]),
            Some(b'z') => {
                self.buf.clear();
                self.buf.resize(4, 0u8);
                Ok(&self.buf[..])
            },
            Some(c1) => {
                let mut val = to_digit(c1)?;
                let mut count = 1;
                for _ in 1..5 {
                    let next = match self.next_in()? {
                        Some(c) => {
                            count += 1;
                            to_digit(c)?
                        },
                        None => 84
                    };
                    val = val * 85 + next;
                }
                let val: u32 = val.try_into()
                    .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidData))?;
                self.buf.clear();
                self.buf.extend_from_slice(&val.to_be_bytes());
                self.buf.truncate(count - 1);
                self.index = 0;
                Ok(&self.buf[..])
            }
        }
    }

    fn consume(&mut self, amt: usize) {
        self.index += amt;
    }
}

fn to_digit(c: u8) -> std::io::Result<u64> {
    if matches!(c, 0x21u8..=0x75u8) {
        Ok((c - 0x21u8) as u64)
    } else {
        Err(std::io::Error::from(std::io::ErrorKind::InvalidData))
    }
}

impl<R: BufRead> Read for Ascii85Decoder<R> {
    fn read(&mut self, out_buf: &mut [u8]) -> std::io::Result<usize> {
        let mut out_index = 0;
        let out_len = out_buf.len();
        while out_index < out_len {
            let in_buf = match self.fill_buf() {
                Ok([]) => return Ok(out_index),
                Ok(buf) => buf,
                Err(err) => match out_index {
                    0 => return Err(err),
                    read => return Ok(read)
                }
            };
            let len = std::cmp::min(in_buf.len(), out_len - out_index);
            out_buf[out_index..(out_index + len)].clone_from_slice(&in_buf[0..len]);
            out_index += len;
            self.consume(len);
        }
        Ok(out_len)
    }
}
