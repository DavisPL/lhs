use std::path::PathBuf;
use std::fs;
use std::env;

fn main() {
    let a = PathBuf::from("foo");
    let x = env::args().nth(1).expect("No argument provided");
    let b = a.join(x);
    fs::write(b, "Hello, world!").expect("Unable to write file");
}
