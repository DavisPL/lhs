

fn main() {
    let a = std::env::args().nth(1).expect("No argument provided");
    std::process::Command::new(a)
        .status()
        .expect("Failed to execute command");
}
