

fn foo(b: String) {
    let a = std::env::args().nth(1).expect("No argument provided");
    let c: String = "foo".to_string();
    std::process::Command::new(c)
        .status()
        .expect("Failed to execute command");
}
