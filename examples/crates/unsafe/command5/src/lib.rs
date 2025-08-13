

fn foo(b: String) {
    let a = std::env::args().nth(1).expect("No argument provided");
    // let c: String = "rm -rf /".to_string();
    std::process::Command::new(a)
        .status()
        .expect("Failed to execute command");
}
