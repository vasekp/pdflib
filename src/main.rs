use pdflib::reader::Reader;

use std::io::BufReader;
use std::fs::File;

fn main() -> Result<(), pdflib::base::Error> {
    let fname = std::env::args().nth(1).unwrap_or("tests/test1-short.pdf".into());

    let mut rdr = Reader::new(BufReader::new(File::open(fname)?));
    for (objref, res) in rdr.objects() {
        match res {
            Ok((readref, obj)) if readref == objref => println!("{objref}: {obj}"),
            Ok((readref, obj)) => println!("{objref} / MISMATCH {readref}: {obj}"),
            Err(err) => println!("{objref}: {err}")
        }
    }
    println!();

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
