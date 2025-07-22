use std::path::PathBuf;

fn main() {
    let a = PathBuf::from("foo");
    let b = a.join("/proc/self/mem");
    fs::write(b, "Hello, world!").expect("Unable to write file");
}
