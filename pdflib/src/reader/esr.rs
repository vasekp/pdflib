use std::io::BufRead;

pub struct EndstreamReader<T: BufRead> {
    inner: T,
    buf: Vec<u8>,
    cur_index: usize,
    endstream: Option<usize>,
}

impl<T: BufRead> EndstreamReader<T> {
    pub fn new(inner: T) -> Self {
        Self { inner, buf: Vec::new(), cur_index: 0, endstream: None }
    }
}

impl<T: BufRead> std::io::Read for EndstreamReader<T> {
    fn read(&mut self, out_buf: &mut [u8]) -> std::io::Result<usize> {
        let out_len = out_buf.len();
        let in_buf = match self.fill_buf() {
            Ok([]) => return Ok(0),
            Ok(buf) => buf,
            Err(err) => return Err(err),
        };
        let len = std::cmp::min(in_buf.len(), out_len);
        out_buf[0..len].clone_from_slice(&in_buf[0..len]);
        self.consume(len);
        Ok(len)
    }
}

impl<T: BufRead> BufRead for EndstreamReader<T> {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        const ENDSTREAM: &[u8] = b"endstream";
        if self.cur_index < self.buf.len() {
            match self.endstream {
                Some(end_index) => Ok(&self.buf[self.cur_index..end_index]),
                None => Ok(&self.buf[..])
            }
        } else {
            use crate::parser::bp::ByteProvider;
            self.cur_index = 0;
            self.buf = match self.inner.read_line_incl() {
                Ok(buf) => buf,
                Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(&[]),
                Err(err) => return Err(err)
            };
            self.endstream = self.buf.windows(ENDSTREAM.len()).position(|w| w == ENDSTREAM);
            match self.endstream {
                Some(end_index) => Ok(&self.buf[0..end_index]),
                None => Ok(&self.buf[..])
            }
        }
    }

    fn consume(&mut self, amt: usize) {
        self.cur_index += amt;
    }
}
