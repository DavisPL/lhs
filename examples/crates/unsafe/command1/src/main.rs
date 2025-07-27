use std::env;

fn main() {
    let a = env::args().nth(1).expect("No argument provided");
    let x = String::from(a);
    std::process::Command::new(&x)
        .status()
        .expect("Failed to execute command");
}