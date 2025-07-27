use std::{fs, io::Read};
use std::env;
fn main() {

    let filename = env::args().nth(1).unwrap_or("dummy".into());
    let x = mystery_function();
    let y = mystery_function();
    if (x + y) * (x + y) == x * x + y * y {
        fs::write(filename, "Unsafe write").unwrap();
    }
    else {
        println!("All good here!");
    }
}

fn mystery_function() -> u8 {
    println!("Who knows what this does? Certainly not Z3.");
    // Read one byte from /dev/urandom
    let mut buffer = [0; 1];
    let mut file = fs::File::open("/dev/urandom").unwrap();
    file.read_exact(&mut buffer).unwrap();
    // Return the byte as a u8
    buffer[0]
}
