use std::io::{BufReader, Read};
use std::fs::File;

use pdflib as pdf;

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
    let mut curr_obj = pdf::Object::Dict(xref.dict.clone());
    curr_obj.print_indented(0);

    //let mut history = vec![];
    for line in std::io::stdin().lines() {
        let line = line?;
        let parts = line.split(' ').collect::<Vec<_>>();
        match parts[..] {
            [p1] => match p1 {
                "stream" => {
                    let pdf::Object::Stream(ref stm) = curr_obj else {
                        log::error!("Not a stream object.");
                        continue;
                    };
                    let data = reader.read_stream_data(&stm)?;
                    let mut read = 0;
                    let mut special = 0;
                    let mut need_nl = true;
                    for c in data.bytes() {
                        let c = c?;
                        match c {
                            0x20..=0x7E | b'\n' => {
                                print!("{}", c as char);
                                read += 1;
                                need_nl = c != b'\n';
                            },
                            _ => {
                                print!("\x1B[7m<{:02x}>\x1B[0m", c);
                                special += 1;
                            }
                        }
                        if read > 1000 || special > 10 {
                            println!("...");
                            need_nl = false;
                            break;
                        }
                    }
                    if need_nl {
                        println!();
                    }
                }
                _ => {}
            },
            [p1, p2] => match (p1.parse::<pdf::ObjNum>(), p2.parse::<pdf::ObjGen>()) {
                (Ok(num), Ok(gen)) => {
                    curr_obj = reader.resolve_ref(&pdf::ObjRef { num, gen })?;
                    curr_obj.print_indented(0);
                }
                _ => {}
            },
            _ => {}
        }
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
        for (key, val) in &self.0 {
            print!("{ind}{}{key} ", Self::SPACES);
            val.print_indented(indent + 1);
        }
        println!("{ind}>>");
   }
}
