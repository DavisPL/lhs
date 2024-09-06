fn main() {
    let x: isize = 0;
    if x == 1 {
        std::fs::write("/proc/self/mem", "this is skipped");
    } else {
        println!("This is printed");
    }
}
