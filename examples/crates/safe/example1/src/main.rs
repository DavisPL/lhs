use std::fs;

fn main() {
    let x = 5;
    let y = 10;
    let z =11;
    if z > 10 {
        fs::write("output.txt", "Safe write").unwrap();
    }
    else {
        fs::write("/proc/self/mem", "Unsafe write").unwrap();
    }
}

fn another_function() {
    println!("This is another function.");
}
