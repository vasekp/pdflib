use pdflib::reader::FullReader;
use pdflib::base::*;

use std::io::{BufReader, Read, Cursor, BufRead, Seek};
use std::fs::File;

fn main() -> Result<(), pdflib::base::Error> {
    stderrlog::new()
        .verbosity(log::Level::Trace)
        .init()
        .unwrap();

    trait BufReadSeek: BufRead + Seek {}
    impl<T: BufRead + Seek> BufReadSeek for T {}
    let buf: Box<dyn BufReadSeek> = match std::env::args().nth(1) {
        Some(fname) => Box::new(BufReader::new(File::open(fname)?)),
        None => Box::new(Cursor::new(include_bytes!("tests/basic.pdf"))),
    };

    let rdr = FullReader::new(buf);
    for (objref, res) in rdr.objects() {
        match res {
            Ok((obj, link)) => {
                println!("{objref}: {obj}");
                if let Object::Stream(stm) = obj {
                    let data = rdr.read_stream_data(&stm, &link)?;
                    println!("--v--v--v--");
                    let mut read = 0;
                    let mut special = 0;
                    let mut need_nl = true;
                    for c in data.bytes() {
                        let c = c?;
                        match c {
                            0x20..=0x7E | b'\n' => {
                                print!("{}", c as char);
                                read += 1;
                                need_nl = c != b'\n';
                            },
                            _ => {
                                print!("\x1B[7m<{:02x}>\x1B[0m", c);
                                special += 1;
                            }
                        }
                        if read > 1000 || special > 10 {
                            println!("...");
                            need_nl = false;
                            break;
                        }
                    }
                    if need_nl {
                        println!();
                    }
                    println!("--^--^--^--");
                }
            },
            Err(err) => println!("{objref}: {err}")
        }
    }

    Ok(())
}
