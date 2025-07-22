fn main() {
    let x = String::from("rm -rf *");
    std::process::Command::new(&x)
        .status()
        .expect("Failed to execute command");
}
