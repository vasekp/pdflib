use std::io::BufRead;

pub trait BufSkip: BufRead {
    fn skip_bytes(&mut self, mut n: usize) -> std::io::Result<()> {
        loop {
            let avail = match self.fill_buf() {
                Ok(n) => n.len(),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e)
            };
            if avail == 0 {
                return Ok(());
            }
            if avail < n {
                self.consume(avail);
                n -= avail;
            } else {
                self.consume(n);
                return Ok(());
            }
        }
    }

    fn skip_to_end(&mut self) -> std::io::Result<()> {
        loop {
            let avail = match self.fill_buf() {
                Ok(n) => n.len(),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e)
            };
            if avail == 0 {
                return Ok(());
            }
            self.consume(avail);
        }
    }
}

impl<T: BufRead> BufSkip for T {}
