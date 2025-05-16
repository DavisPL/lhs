fn main() {
    let x: isize = 1;
    if x == 1 {
        std::fs::write("/proc/self/mem", "dangerous write");
    } else {
        println!("Harmless print");
    }
}
