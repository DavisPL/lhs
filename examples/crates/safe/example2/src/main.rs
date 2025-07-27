use std::fs;
use std::env;

fn get_random_bool() -> bool {
    let random_value = rand::random::<u8>();
    random_value % 2 == 0
}

fn main() {

    let filename = env::args().nth(1).unwrap_or("dummy".into());
    let mut x = 0;
    let b = get_random_bool();
    if b {
        x += 5;
    } else {
        x += 3;
    }
    if x > 5 {
        fs::write(filename, "Unsafe write").unwrap();
    }
}