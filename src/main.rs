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
                let data = parser.read_stream_data(offset, None)?;
                println!("{offset} + {} bytes (incl. EOF)", data.len());

                let Some(len_obj) = dict.lookup(b"Length") else { continue };
                let len = match *len_obj {
                    Object::Number(Number::Int(len)) => len,
                    Object::Ref(oref @ ObjRef(num, gen)) => {
                        let Some(rec) = xref.table.get(&num) else {
                            return Err(Error::Parse("Length object not found"))
                        };
                        let &Record::Used{gen: g2, offset: len_off} = rec else {
                            return Err(Error::Parse("Length object not found"))
                        };
                        if g2 != gen {
                            return Err(Error::Parse("Length object not found"))
                        };
                        match parser.read_obj_at(len_off, &oref)? {
                            Object::Number(Number::Int(len)) => len,
                            _ => return Err(Error::Parse("Length object of wrong type"))
                        }
                    },
                    _ => return Err(Error::Parse("Length of wrong type"))
                };
                let data = parser.read_stream_data(offset, Some(len))?;
                println!("{offset} + {} bytes (exact)", data.len());
            }
        },
        _ => todo!()
    }
    Ok(())
}
