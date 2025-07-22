fn main() {
    let x = String::from("ls");
    std::process::Command::new(&x)
        .status()
        .expect("Failed to execute command");
   
}