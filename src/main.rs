use pdflib::reader::Reader;
use pdflib::base::*;

use std::io::BufReader;
use std::io::Read;
use std::fs::File;

fn main() -> Result<(), pdflib::base::Error> {
    stderrlog::new()
        .verbosity(log::Level::Trace)
        .init()
        .unwrap();

    let fname = std::env::args().nth(1).unwrap_or("tests/basic.pdf".into());

    let rdr = Reader::new(BufReader::new(File::open(fname)?));
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
