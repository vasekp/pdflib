use pdflib::parser::Parser;

use std::io::BufReader;
use std::fs::File;

fn main() -> std::io::Result<()> {
    let f = File::open("tests/test1-short.pdf")?;
    let mut parser = Parser::new(BufReader::new(f));
    parser.seek_to(335)?;
    println!("{}", parser.read_obj()?);
    println!("{}", parser.locate_trailer()?);
    Ok(())
}
