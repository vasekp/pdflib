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
            for (num, rec) in xref.table {
                let Record::Used{gen, offset} = rec else { continue };
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
                    },
                    _ => unimplemented!()
                }
            }
        },
        _ => todo!()
    }
    Ok(())
}
