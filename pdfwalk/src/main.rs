use std::io::BufReader;
use std::fs::File;

use pdflib as pdf;
use pdf::Resolver;

fn main() -> Result<(), pdf::Error> {
    stderrlog::new()
        .verbosity(log::Level::Trace)
        .init()
        .unwrap();

    let Some(fname) = std::env::args().nth(1) else {
        println!("Usage: {} filename", std::env::args().next().unwrap());
        return Ok(())
    };

    let file = File::open(fname)?;
    let reader = pdf::reader::SimpleReader::new(BufReader::new(file))?;
    let xref = &reader.xref;
    let trailer = || pdf::Object::Dict(xref.dict.clone());
    let mut history = vec![];
    let mut curr_obj = trailer();
    curr_obj.print_indented(0);

    let root_ref = xref.dict.lookup(b"Root")
        .as_objref()
        .ok_or(pdf::Error::Parse("Could not find /Root."))?;
    let root = reader.resolve_ref(root_ref)?
        .into_dict()
        .ok_or(pdf::Error::Parse("Could not find /Root."))?;

    for line in std::io::stdin().lines() {
        let line = line?;
        let parts = line.split(' ').collect::<Vec<_>>();
        match parts[..] {
            ["top"] => {
                history.clear();
                curr_obj = trailer();
            },
            ["root"] => {
                history.clear();
                history.push(*root_ref);
                curr_obj = reader.resolve_ref(root_ref)?;
            },
            ["up"] => {
                history.pop();
                if let Some(objref) = history.last() {
                    curr_obj = reader.resolve_ref(objref)?;
                } else {
                    curr_obj = trailer();
                }
            },
            ["stream"] => {
                let Some(stm) = curr_obj.as_stream() else {
                    log::error!("Not a stream object.");
                    continue;
                };
                let mut data = reader.read_stream_data(stm)?;
                let mut cmd = std::process::Command::new("less")
                    .stdin(std::process::Stdio::piped())
                    .arg("-R")
                    .spawn()?;
                let mut stdin = cmd.stdin.as_ref().unwrap();
                std::io::copy(&mut data, &mut stdin)?;
                cmd.wait()?;
            },
            ["page", p2] => {
                let Ok(page_num) = p2.parse::<usize>() else {
                    log::error!("Malformed page number.");
                    continue;
                };
                let objref = find_page(&reader, &root, page_num)?;
                println!("{}", objref);
                curr_obj = reader.resolve_ref(&objref)?;
                history.push(objref);
            },
            [p1, p2] => {
                let (Ok(num), Ok(gen)) = (p1.parse::<pdf::ObjNum>(), p2.parse::<pdf::ObjGen>()) else {
                    log::error!("Could not parse as a object reference.");
                    continue;
                };
                let objref = pdf::ObjRef { num, gen };
                curr_obj = reader.resolve_ref(&objref)?;
                history.push(objref);
            },
            _ => log::error!("Unknown command.")
        }
        curr_obj.print_indented(0);
    }

    Ok(())
}

trait PrettyPrint {
    const SPACES: &str = "  ";

    fn print_indented(&self, indent: usize);
}

impl PrettyPrint for pdf::Object {
    fn print_indented(&self, indent: usize) {
        let ind = Self::SPACES.repeat(indent);
        match self {
            pdf::Object::Array(arr) => arr.print_indented(indent),
            pdf::Object::Dict(dict) => dict.print_indented(indent),
            pdf::Object::Stream(stm) => {
                stm.dict.print_indented(indent);
                println!("{ind}[stream]");
            },
            obj => println!("{obj}")
        }
    }
}

impl PrettyPrint for Vec<pdf::Object> {
    fn print_indented(&self, indent: usize) {
        let ind = Self::SPACES.repeat(indent);
        println!("[");
        for item in self {
            print!("{ind}{}", Self::SPACES);
            item.print_indented(indent + 1);
        }
        println!("{ind}]");
    }
}

impl PrettyPrint for pdf::Dict {
    fn print_indented(&self, indent: usize) {
        let ind = Self::SPACES.repeat(indent);
        println!("<<");
        for (key, val) in self.as_slice() {
            print!("{ind}{}{key} ", Self::SPACES);
            val.print_indented(indent + 1);
        }
        println!("{ind}>>");
   }
}

fn find_page(reader: &pdf::reader::SimpleReader<BufReader<File>>, root: &pdf::Dict,
    page_num: usize) -> Result<pdf::ObjRef, pdf::Error> {
    let Some(mut num) = page_num.checked_sub(1) else {
        return Err(pdf::Error::Parse("Page number out of range."));
    };
    let mut curr_ref = *root.lookup(b"Pages").as_objref()
        .ok_or(pdf::Error::Parse("Could not find /Pages."))?;
    let mut curr_node = reader.resolve_ref(&curr_ref)?.into_dict()
        .ok_or(pdf::Error::Parse("Could not find /Pages."))?;
    let mut count = curr_node.lookup(b"Count").num_value()
        .ok_or(pdf::Error::Parse("Could not read page tree."))?;
    let err = || pdf::Error::Parse("Could not read page tree.");
    'a: loop {
        if num >= count {
            return Err(pdf::Error::Parse("Page number out of range."));
        }
        let kids = reader.resolve_obj(curr_node.lookup(b"Kids").to_owned())?.into_array().ok_or(err())?;
        if kids.len() == count {
            return Ok(*kids[num].as_objref().ok_or(err())?);
        }
        for kid in kids {
            let objref = *kid.as_objref().ok_or(err())?;
            let node = reader.resolve_ref(&objref)?.into_dict().ok_or(err())?;
            if node.lookup(b"Parent") != &pdf::Object::Ref(curr_ref) {
                return Err(pdf::Error::Parse("malformed page tree (/Parent)"));
            }
            let this_count = match node.lookup(b"Type")
                .as_name()
                .ok_or(pdf::Error::Parse("malformed page tree"))?
                .as_slice() {
                    b"Pages" =>
                        node.lookup(b"Count").num_value()
                            .ok_or(pdf::Error::Parse("Could not read page tree."))?,
                    b"Page" => 1,
                    _ => return Err(pdf::Error::Parse("malformed page tree (/Type)"))
            };
            if this_count > num {
                count = this_count;
                curr_ref = objref;
                curr_node = node;
                continue 'a;
            } else {
                num -= this_count;
            }
        }
    }
}
