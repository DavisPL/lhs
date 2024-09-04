fn main() {
    let x: isize = 1;
    if x == 1 {
        std::fs::write("/proc/self/mem", "soba");
    } else {
        println!("I'm sad");
    }
}
