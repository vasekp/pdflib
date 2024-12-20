use pdflib::parser::Parser;
use pdflib::base::Record;

use std::io::BufReader;
use std::fs::File;

fn main() -> Result<(), pdflib::base::Error> {
    let f = File::open("tests/test1-short.pdf")?;
    let mut parser = Parser::new(BufReader::new(f));
    let xref = parser.entrypoint()?;
    println!("{}", xref.trailer);
    for (num, rec) in xref.table {
        let Record::Used{gen, offset} = rec else { continue };
        parser.seek_to(offset)?;
        println!("{num} {gen}: {}", parser.read_obj_indirect(num, gen)?);
    }
    Ok(())
}
