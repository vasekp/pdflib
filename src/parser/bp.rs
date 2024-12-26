use std::io::{BufRead, Seek};

pub trait ByteProvider: BufRead + Seek {
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

    fn read_line(&mut self) -> std::io::Result<Vec<u8>> {
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
                    let crlf = buf[pos] == b'\r' && buf.len() > pos && buf[pos + 1] == b'\n';
                    self.consume(pos + if crlf { 2 } else { 1 });
                    break;
                },
                None => {
                    line.extend_from_slice(buf);
                    let len = buf.len();
                    self.consume(len);
                }
            }
        }
        Ok(line)
    }

    fn skip_past_eol(&mut self) -> std::io::Result<()> {
        loop {
            let buf = match self.fill_buf() {
                Ok(buf) => buf,
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err)
            };
            if buf.is_empty() {
                return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
            }
            match buf.iter().position(|c| *c == b'\n' || *c == b'\r') {
                Some(pos) => {
                    let crlf = buf[pos] == b'\r' && buf.len() > pos && buf[pos + 1] == b'\n';
                    self.consume(pos + if crlf { 2 } else { 1 });
                    return Ok(());
                },
                None => {
                    let len = buf.len();
                    self.consume(len);
                }
            }
        }
    }
}

impl<T: BufRead + Seek> ByteProvider for T { }


#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_read_line() {
        let mut bytes = Cursor::new("line 1\nline 2\rline 3\r\nline 4\n\rline 5");
        assert_eq!(ByteProvider::read_line(&mut bytes).unwrap(), b"line 1");
        assert_eq!(ByteProvider::read_line(&mut bytes).unwrap(), b"line 2");
        assert_eq!(ByteProvider::read_line(&mut bytes).unwrap(), b"line 3");
        assert_eq!(ByteProvider::read_line(&mut bytes).unwrap(), b"line 4");
        assert_eq!(ByteProvider::read_line(&mut bytes).unwrap(), b"");
        assert_eq!(ByteProvider::read_line(&mut bytes).unwrap(), b"line 5");
        assert!(ByteProvider::read_line(&mut bytes).is_err());

        let mut bytes = Cursor::new("line 1\nline 2\rline 3\r\nline 4\n\rline 5");
        bytes.skip_past_eol().unwrap();
        bytes.skip_past_eol().unwrap();
        bytes.skip_past_eol().unwrap();
        bytes.skip_past_eol().unwrap();
        assert_eq!(ByteProvider::read_line(&mut bytes).unwrap(), b"");
        assert!(ByteProvider::skip_past_eol(&mut bytes).is_err());
    }
}
