use std::env;

fn main() {
    let a = env::args().nth(1).expect("No argument provided");

    let x = String::from("rm -rf *");

    if a == "0"{
        std::process::Command::new(x)
            .status()
            .expect("Failed to execute command");
    }
}