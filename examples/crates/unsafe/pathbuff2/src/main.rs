use std::path::PathBuf;

fn main() {
    let a = PathBuf::from("/proc");
    let b = a.join("self/mem");
    fs::write(b, "Hello, world!").expect("Unable to write file");
}
