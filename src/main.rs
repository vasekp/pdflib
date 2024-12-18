use pdflib::parser::ObjParser;

use std::io::BufReader;
use std::fs::File;

fn main() -> std::io::Result<()> {
    let f = File::open("tests/test1-short.pdf")?;
    let mut tkn = ObjParser::new(BufReader::new(f));
    tkn.seek_to(335)?;
    println!("{:?}", tkn.read_obj()?);
    Ok(())
}
