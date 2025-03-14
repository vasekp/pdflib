use std::io::{Read, BufRead, BufReader};
use flate2::bufread::ZlibDecoder;
use crate::base::Dict;

pub fn decode<'a, R: BufRead + 'a>(input: R, params: &Dict) -> Box<dyn BufRead + 'a> {
    match params.lookup(b"Predictor").num_value() {
        None => Box::new(BufReader::new(ZlibDecoder::new(input))),
        Some(10..=15) => Box::new(PNGDecode::new(
            ZlibDecoder::new(input),
            params.lookup(b"Columns").num_value().unwrap_or(1),
        )),
        _ => unimplemented!(),
    }
}

struct PNGDecode<R: Read> {
    input: R,
    cols: usize,
    prev_row: Vec<u8>,
    index: usize
}

impl<R: Read> PNGDecode<R> {
    fn new(input: R, cols: usize) -> Self {
        PNGDecode { input, cols, prev_row: Vec::new(), index: 0 }
    }

    fn read_row(&mut self) -> std::io::Result<&[u8]> {
        let mut enc_row = vec![0; 1 + self.cols];
        if let Err(err) = self.input.read_exact(&mut enc_row) {
            match err.kind() {
                std::io::ErrorKind::UnexpectedEof => return Ok(&[]),
                _ => return Err(err)
            }
        }
        let (enc, in_row) = enc_row.split_first().unwrap(); // size >= 1 always
        let mut prev_row = std::mem::take(&mut self.prev_row);
        if prev_row.is_empty() {
            prev_row.resize(self.cols, 0);
        }
        let new_row = &mut self.prev_row;
        match enc {
            0 => new_row.extend_from_slice(in_row),
            1 => {
                let mut out_val = 0u8;
                for in_val in in_row {
                    out_val = out_val.overflowing_add(*in_val).0;
                    new_row.push(out_val);
                }
            },
            2 => {
                for (old_val, new_val) in std::iter::zip(prev_row, in_row) {
                    let out_val = old_val.overflowing_add(*new_val).0;
                    new_row.push(out_val);
                }
            },
            _ => unimplemented!("PNG predictor {enc}")
        }
        self.index = 0;
        Ok(&self.prev_row)
    }
}

impl<R: Read> BufRead for PNGDecode<R> {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        if self.index < self.prev_row.len() {
            Ok(&self.prev_row[self.index..])
        } else {
            self.read_row()
        }
    }

    fn consume(&mut self, amt: usize) {
        self.index += amt;
    }
}

impl<R: Read> Read for PNGDecode<R> {
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
