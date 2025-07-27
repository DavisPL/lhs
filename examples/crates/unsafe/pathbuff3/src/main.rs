use std::path::PathBuf;
use std::fs;
use std::env;

fn main() {
    let x = env::args().nth(1).expect("No argument provided");
    let a = PathBuf::from(x);
    let b = a.join("");
    fs::write(b, "Hello, world!").expect("Unable to write file");
}
