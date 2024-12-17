use pdflib::parser::ObjParser;

fn main() -> std::io::Result<()> {
    let mut tkn = ObjParser::from("<</Length 8 0 R>>");
    let obj = tkn.read_obj()?;
    println!("{:?}", obj);
    Ok(())
}
