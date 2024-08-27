fn main() {
    let filename = "a.txt";
    let contents = "Hello, world!";
    // std::fs::write("a.txt", "Hello, world!").unwrap();
    std::fs::write(filename, contents).unwrap();
    // println!("Hello, world!");
}
