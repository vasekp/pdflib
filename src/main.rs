use pdflib::parser::Parser;

use std::io::BufReader;
use std::fs::File;

fn main() -> Result<(), pdflib::base::Error> {
    let f = File::open("tests/test1-short.pdf")?;
    let mut parser = Parser::new(BufReader::new(f));
    println!("{}", parser.entrypoint()?.trailer);
    Ok(())
}
