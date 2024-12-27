use pdflib::parser::Parser;
use pdflib::base::*;

use std::io::BufReader;
use std::fs::File;

fn main() -> Result<(), pdflib::base::Error> {
    let f = File::open("tests/test1-short.pdf")?;
    let mut parser = Parser::new(BufReader::new(f));
    let entry = parser.entrypoint()?;
    parser.seek_to(entry)?;
    match parser.read_obj_toplevel()? {
        TLO::XRef(xref) => {
            println!("{}", xref.trailer);
            for (&num, rec) in &xref.table {
                let &Record::Used{gen, offset} = rec else { continue };
                parser.seek_to(offset)?;
                match parser.read_obj_toplevel()? {
                    TLO::IndirObject(ObjRef(n2, g2), obj) => {
                        assert_eq!(n2, num);
                        assert_eq!(g2, gen);
                        println!("{num} {gen}: {}", obj);
                    },
                    TLO::Stream(ObjRef(n2, g2), stm) => {
                        assert_eq!(n2, num);
                        assert_eq!(g2, gen);
                        let Stream{dict, data: Data::Ref(offset)} = stm else { panic!() };
                        println!("{num} {gen}: {dict} stream");
                        let data = parser.read_stream_data(offset, None)?;
                        println!("{offset} + {} bytes (incl. EOF)", data.len());

                        let Some(len_obj) = dict.lookup(b"Length") else { continue };
                        let len = match *len_obj {
                            Object::Number(Number::Int(len)) => len,
                            Object::Ref(ObjRef(num, gen)) => {
                                let Some(rec) = xref.table.get(&num) else {
                                    return Err(Error::Parse("Length object not found"))
                                };
                                let &Record::Used{gen: g2, offset: len_off} = rec else {
                                    return Err(Error::Parse("Length object not found"))
                                };
                                if g2 != gen {
                                    return Err(Error::Parse("Length object not found"))
                                };
                                parser.seek_to(len_off)?;
                                match parser.read_obj_toplevel()? {
                                    TLO::IndirObject(ObjRef(n2, g2),
                                        Object::Number(Number::Int(len)))
                                        if n2 == num && g2 == gen
                                            => len,
                                    _ => return Err(Error::Parse("Length object of wrong type"))
                                }
                            },
                            _ => return Err(Error::Parse("Length of wrong type"))
                        };
                        let data = parser.read_stream_data(offset, Some(len))?;
                        println!("{offset} + {} bytes (exact)", data.len());
                    },
                    _ => unimplemented!()
                }
            }
        },
        _ => todo!()
    }
    Ok(())
}
