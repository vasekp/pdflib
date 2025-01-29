use pdflib::reader::Reader;
use pdflib::base::*;

use std::io::BufReader;
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
                if let Object::Stream(Stream { dict, .. }) = obj {
                    println!("Length: {}", rdr.resolve_deep(dict.lookup(b"Length"), &link)?);
                    println!("Filter: {}", rdr.resolve_deep(dict.lookup(b"Filter"), &link)?);
                }
            },
            Err(err) => println!("{objref}: {err}")
        }
    }

    /*for (oref, offset) in recs {
        let obj = parser.read_obj_at(offset, &oref)?;
        println!("{} {}: {}", oref.num, oref.gen, obj);
        let Object::Stream(stm) = obj else { continue };
        let Stream{dict, data: Data::Ref(offset)} = stm else { panic!() };
        let len_obj = dict.lookup(b"Length");
        let data_raw: Box<dyn Read> = match *len_obj {
            Object::Number(Number::Int(len)) if len > 0 => {
                println!("{offset} + {} bytes (exact)", len);
                Box::new(parser.read_raw(offset)?.take(len as u64))
            },
            _ => {
                println!("{offset} + unknown length"); // TODO: endstream
                Box::new(parser.read_raw(offset)?)
            },
        };
        let data = BufReader::new(codecs::decode(data_raw, dict.lookup(b"Filter")));
        println!("-----");
        for c in data.bytes() {
            let c = c?;
            match c {
                0x20..=0x7E | b'\n' => print!("{}", c as char),
                _ => print!("\x1B[7m<{:02x}>\x1B[0m", c)
            }
        }
        println!("\n-----\n");
    }*/

    Ok(())
}
