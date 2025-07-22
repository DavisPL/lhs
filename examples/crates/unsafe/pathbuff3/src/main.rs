use std::path::PathBuf;

fn main() {
    let a = PathBuf::from("/proc/self/mem");
    let b = a.join("");
    fs::write(b, "Hello, world!").expect("Unable to write file");
}
