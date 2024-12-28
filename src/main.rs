use pdflib::parser::Parser;
use pdflib::base::*;

use std::io::BufReader;
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
                println!("{num} {gen}: {dict} stream");

                let mut len_obj = dict.lookup(b"Length").unwrap_or(&Object::Null);
                let resolved;
                if let Object::Ref(oref) = len_obj {
                    resolved = parser.find_obj(&oref, &xref)?;
                    len_obj = &resolved;
                }
                match *len_obj {
                    Object::Number(Number::Int(len)) => {
                        let data = parser.read_stream_data(offset, Some(len))?;
                        println!("{offset} + {} bytes (exact)", data.len());
                    },
                    Object::Null => {
                        let data = parser.read_stream_data(offset, None)?;
                        println!("{offset} + {} bytes (incl. EOF)", data.len());
                    },
                    _ => return Err(Error::Parse("Length object of wrong type"))
                }
            }
        },
        _ => todo!()
    }
    Ok(())
}
