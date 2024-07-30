pub mod symbolic;

use std::fs;
use std::io::{self, ErrorKind};
use std::path::PathBuf;

fn main() {
    println!("Hello, world!");
}

/// This function does contain an input (`filename: /proc/self/mem`) such that a safety property (write to /proc/self/mem) is violated.
pub fn write_to_file(contents: &str, filename: &str) -> io::Result<()> {
    fs::write(contents, filename)?;
    Ok(())
}

/// This function also contains an input (filename: `/proc/../proc/self/mem`) such that a safety property (write to /proc/self/mem) is violated.
pub fn write_to_file_safe(contents: &str, filename: &str) -> io::Result<()> {
    if filename == "/proc/self/mem" {
        return Err(io::Error::new(ErrorKind::Other, "Unsafe write!"));
    }
    fs::write(contents, filename)?;
    Ok(())
}

/// This function contains no input such that a write to /proc/self/mem occurs.
pub fn write_to_file_actually_safe(contents: &str, filename: &str) -> io::Result<()> {
    let filename_realpath: PathBuf = fs::canonicalize(filename)?;
    let unsafe_path: PathBuf = fs::canonicalize("/proc/self/mem")?;
    if filename_realpath == unsafe_path {
        return Err(io::Error::new(ErrorKind::Other, "Unsafe write!"));
    }
    fs::write(contents, filename)?;
    Ok(())
}

/// This function contains an input (operation: `|x| {fs::write("hello", "/proc/self/mem"); x}`)such that a write to /proc/self/mem occurs.
pub fn apply_operation_twice(num: i32, operation: impl Fn(i32) -> i32) -> i32 {
    operation(operation(num))
}

/// This function contains no input such that a write to /proc/self/mem occurs.
pub fn write_to_hw3(contents: &str) -> io::Result<()> {
    fs::write("~/caleb/hw3/src/main.rs", contents)
}
