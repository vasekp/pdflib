use pdflib::parser::Parser;
use pdflib::base::*;

use std::io::{BufReader, Read, Write};
use std::fs::File;

fn main() -> Result<(), pdflib::base::Error> {
    let f = File::open("tests/test1-short.pdf")?;
    let mut parser = Parser::new(BufReader::new(f));
    let entry = parser.entrypoint()?;
    match parser.read_xref_at(entry)? {
        XRef::Table(xref) => {
            println!("{}", xref.trailer);
            for (&num, rec) in &xref.table {
                let &Record::Used{gen, offset} = rec else { continue };
                let obj = parser.read_obj_at(offset, &ObjRef(num, gen))?;
                println!("{num} {gen}: {}", obj);
                let Object::Stream(stm) = obj else { continue };
                let Stream{dict, data: Data::Ref(offset)} = stm else { panic!() };
                let mut len_obj = dict.lookup(b"Length").unwrap_or(&Object::Null);
                let resolved;
                if let Object::Ref(oref) = len_obj {
                    resolved = parser.find_obj(oref, &xref)?;
                    len_obj = &resolved;
                }
                let data_raw = match *len_obj {
                    Object::Number(Number::Int(len)) => {
                        let data = parser.read_stream_data(offset, Some(len))?;
                        println!("{offset} + {} bytes (exact)", data.len());
                        data
                    },
                    Object::Null => {
                        let data = parser.read_stream_data(offset, None)?;
                        println!("{offset} + {} bytes (incl. EOF)", data.len());
                        data
                    },
                    _ => return Err(Error::Parse("Length object of wrong type"))
                };
                assert_eq!(dict.lookup(b"Filter"), Some(&Object::new_name("FlateDecode")));
                use flate2::bufread::ZlibDecoder;
                let mut deflater = ZlibDecoder::new(&data_raw[..]);
                let mut data_dec = Vec::new();
                deflater.read_to_end(&mut data_dec)?;
                println!();
                std::io::stdout().write_all(&data_dec)?;
                println!("\n({} bytes read)", deflater.total_out());
            }
        },
        _ => todo!()
    }
    Ok(())
}
