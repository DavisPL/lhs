
fn foo(x: String) {
    let y = String::from("rm -rf /");
    std::process::Command::new(y)
        .status()
        .expect("failed to execute command");
}
