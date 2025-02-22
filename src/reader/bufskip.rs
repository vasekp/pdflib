use std::io::BufRead;

pub trait BufSkip: BufRead {
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
