use std::io::BufReader;
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
                    let mut data = reader.read_stream_data(&stm)?;
                    let mut cmd = std::process::Command::new("less")
                        .stdin(std::process::Stdio::piped())
                        .arg("-R")
                        .spawn()?;
                    let mut stdin = cmd.stdin.as_ref().unwrap();
                    std::io::copy(&mut data, &mut stdin)?;
                    cmd.wait()?;
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
