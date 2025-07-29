use std::path::PathBuf;
use std::fs;
use std::env;

fn main() {
    let a = PathBuf::from("/proc/self/mem");
    let b = a.join("");
    fs::write(b, "Hello, world!").expect("Unable to write file");
}
