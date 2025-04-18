use std::io::BufReader;
use std::fs::File;

use pdflib as pdf;
use pdf::Resolver;

macro_rules! try_or_continue {
    ($expr:expr) => {
        match $expr {
            Ok(val) => val,
            Err(pdf::Error::Parse(err)) => {
                log::error!("{err}");
                continue;
            },
            Err(err) => return Err(err)
        }
    }
}

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

    'main: for line in std::io::stdin().lines() {
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
                curr_obj = try_or_continue!(reader.resolve_ref(root_ref));
            },
            ["up"] => {
                history.pop();
                if let Some(objref) = history.last() {
                    curr_obj = try_or_continue!(reader.resolve_ref(objref));
                } else {
                    curr_obj = trailer();
                }
            },
            ["stream"] => {
                let Some(stm) = curr_obj.as_stream() else {
                    log::error!("Not a stream object.");
                    continue;
                };
                let mut data = try_or_continue!(reader.read_stream_data(stm));
                let mut cmd = std::process::Command::new("less")
                    .stdin(std::process::Stdio::piped())
                    .arg("-R")
                    .spawn()?;
                let mut stdin = cmd.stdin.as_ref().unwrap();
                match std::io::copy(&mut data, &mut stdin) {
                    Ok(_) => { cmd.wait()?; },
                    Err(err) => {
                        cmd.kill()?;
                        log::error!("{err}");
                    }
                }
            },
            ["page", p2] => {
                let Ok(page_num) = p2.parse::<usize>() else {
                    log::error!("Malformed page number.");
                    continue;
                };
                let objref = try_or_continue!(find_page(&reader, &root, page_num));
                println!("{}", objref);
                curr_obj = try_or_continue!(reader.resolve_ref(&objref));
                history.push(objref);
            },
            [part, ref rest @ ..] if part.starts_with('/') || part.starts_with('[') => {
                let mut subobj = &curr_obj;
                for spec in std::iter::once(part).chain(rest.into_iter().map(std::ops::Deref::deref)) {
                    let bs = spec.as_bytes();
                    match bs[0] {
                        b'/' => {
                            let dict = match subobj {
                                pdf::Object::Dict(ref dict) | pdf::Object::Stream(pdf::Stream { ref dict, .. }) => dict,
                                _ => {
                                    log::error!("{subobj}: Not a dictionary.");
                                    continue 'main;
                                }
                            };
                            subobj = dict.lookup(&bs[1..]);
                        },
                        b'[' => {
                            if bs[bs.len() - 1] != b']' {
                                log::error!("Malformed command.");
                                continue 'main;
                            }
                            let index = match std::str::from_utf8(&bs[1..(bs.len() - 1)])
                                    .expect("string from stdin should have a valid UTF substring")
                                    .parse::<usize>() {
                                Ok(num) => num,
                                Err(_) => {
                                    log::error!("Malformed command.");
                                    continue 'main;
                                }
                            };
                            if index == 0 {
                                log::error!("Malformed index (should be 1-based).");
                                continue 'main;
                            }
                            let pdf::Object::Array(ref arr) = subobj else {
                                log::error!("{subobj}: Not an array.");
                                continue 'main;
                            };
                            subobj = arr.get(index - 1).unwrap_or(&pdf::Object::Null);
                        },
                        _ => {
                            log::error!("Malformed command.");
                            continue 'main;
                        }
                    }
                }
                if let &pdf::Object::Ref(objref) = subobj {
                    curr_obj = try_or_continue!(reader.resolve_ref(&objref));
                    history.push(objref);
                } else {
                    subobj.print_indented(0);
                    continue;
                }
            },
            [p1, p2] => {
                let (Ok(num), Ok(gen)) = (p1.parse::<pdf::ObjNum>(), p2.parse::<pdf::ObjGen>()) else {
                    log::error!("Could not parse as a object reference.");
                    continue;
                };
                let objref = pdf::ObjRef { num, gen };
                curr_obj = try_or_continue!(reader.resolve_ref(&objref));
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
        for (index, item) in self.iter().enumerate() {
            print!("{ind}{}[{}] ", Self::SPACES, index + 1);
            item.print_indented(indent + 1);
        }
        println!("{ind}]");
    }
}

impl PrettyPrint for pdf::Dict {
    fn print_indented(&self, indent: usize) {
        let ind = Self::SPACES.repeat(indent);
        println!("<<");
        for (key, val) in self.iter() {
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
