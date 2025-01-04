use pdflib::parser::Parser;
use pdflib::base::*;
use pdflib::codecs;

use std::io::{BufReader, Read, Cursor};
use std::fs::File;

fn main() -> Result<(), pdflib::base::Error> {
    let fname = std::env::args().nth(1).unwrap_or("tests/test1-short.pdf".into());
    println!("{fname}");
    let f = File::open(fname)?;
    let mut parser = Parser::new(BufReader::new(f));
    let entry = parser.entrypoint()?;

    let (tpe, mut iter) = parser.read_xref_at(entry)?;
    println!("xref at {entry} ({})", match tpe {
        XRefType::Table => "table",
        XRefType::Stream(_) => "stream"
    });
    let mut recs = Vec::new();
    for res in &mut iter {
        let (num, rec) = res?;
        let Record::Used{gen, offset} = rec else { continue };
        recs.push((ObjRef{num, gen}, offset));
    }
    println!("{}", iter.trailer()?);
    for (oref, offset) in recs {
        let obj = parser.read_obj_at(offset, &oref)?;
        println!("{} {}: {}", oref.num, oref.gen, obj);
        let Object::Stream(stm) = obj else { continue };
        let Stream{dict, data: Data::Ref(offset)} = stm else { panic!() };
        let len_obj = dict.lookup(b"Length").unwrap_or(&Object::Null);
        let data_raw = match *len_obj {
            Object::Number(Number::Int(len)) => {
                let data = parser.read_stream_data(offset, Some(len))?;
                println!("{offset} + {} bytes (exact)", data.len());
                data
            },
            _ => {
                let data = parser.read_stream_data(offset, None)?;
                println!("{offset} + {} bytes (incl. EOL)", data.len());
                data
            },
        };
        let mut deflater = codecs::decode(Cursor::new(data_raw),
            dict.lookup(b"Filter").unwrap_or(&Object::Null));
        let mut data_dec = Vec::new();
        deflater.read_to_end(&mut data_dec)?;
        println!("-----");
        for c in data_dec {
            match c {
                0x20..=0x7E | b'\n' => print!("{}", c as char),
                _ => print!("\x1B[7m<{:02x}>\x1B[0m", c)
            }
        }
        println!("\n-----\n");
    }
    Ok(())
}
