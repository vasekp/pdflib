use std::io::BufRead;

pub trait ByteProvider: BufRead {
    fn peek(&mut self) -> Option<u8> {
        match self.fill_buf() {
            Ok(buf) => Some(buf[0]),
            _ => None
        }
    }

    fn next_or_eof(&mut self) -> std::io::Result<u8> {
        let buf = self.fill_buf()?;
        if !buf.is_empty() {
            let ret = buf[0];
            self.consume(1);
            Ok(ret)
        } else {
            Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof))
        }
    }

    fn next_if(&mut self, cond: impl FnOnce(u8) -> bool) -> Option<u8> {
        let buf = self.fill_buf().ok()?;
        if !buf.is_empty() && cond(buf[0]) {
            let ret = buf[0];
            self.consume(1);
            Some(ret)
        } else {
            None
        }
    }

    fn read_line_inner(&mut self, include_eol: bool) -> std::io::Result<Vec<u8>> {
        let mut line = Vec::new();
        loop {
            let buf = match self.fill_buf() {
                Ok(buf) => buf,
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err)
            };
            if buf.is_empty() {
                if line.is_empty() {
                    return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
                } else {
                    break;
                }
            }
            match buf.iter().position(|c| *c == b'\n' || *c == b'\r') {
                Some(pos) => {
                    line.extend_from_slice(&buf[..pos]);
                    self.consume(pos);
                    let buf = loop {
                        match self.fill_buf() {
                            Ok(buf) => break buf,
                            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                            Err(err) => return Err(err)
                        }
                    };
                    let crlf = buf[0] == b'\r' && buf.len() > 1 && buf[1] == b'\n';
                    let eol_len = if crlf { 2 } else { 1 };
                    if include_eol {
                        line.extend_from_slice(&buf[0..eol_len]);
                    }
                    self.consume(eol_len);
                    break;
                },
                None => {
                    line.extend_from_slice(buf);
                    let len = buf.len();
                    self.consume(len);
                }
            }
        }
        line.shrink_to_fit();
        Ok(line)
    }

    fn read_line_excl(&mut self) -> std::io::Result<Vec<u8>> {
        self.read_line_inner(false)
    }

    fn read_line_incl(&mut self) -> std::io::Result<Vec<u8>> {
        self.read_line_inner(true)
    }
}

impl<T: BufRead> ByteProvider for T { }


#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_read_line() {
        let mut bytes = Cursor::new("line 1\nline 2\rline 3\r\nline 4\n\rline 5");
        assert_eq!(bytes.read_line_excl().unwrap(), b"line 1");
        assert_eq!(bytes.read_line_excl().unwrap(), b"line 2");
        assert_eq!(bytes.read_line_excl().unwrap(), b"line 3");
        assert_eq!(bytes.read_line_excl().unwrap(), b"line 4");
        assert_eq!(bytes.read_line_excl().unwrap(), b"");
        assert_eq!(bytes.read_line_excl().unwrap(), b"line 5");
        assert!(bytes.read_line_excl().is_err());

        let mut bytes = Cursor::new("line 1\nline 2\rline 3\r\nline 4\n\rline 5");
        assert_eq!(bytes.read_line_incl().unwrap(), b"line 1\n");
        assert_eq!(bytes.read_line_incl().unwrap(), b"line 2\r");
        assert_eq!(bytes.read_line_incl().unwrap(), b"line 3\r\n");
        assert_eq!(bytes.read_line_incl().unwrap(), b"line 4\n");
        assert_eq!(bytes.read_line_incl().unwrap(), b"\r");
        assert_eq!(bytes.read_line_incl().unwrap(), b"line 5");
        assert!(bytes.read_line_incl().is_err());
    }
}
