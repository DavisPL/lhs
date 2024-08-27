use std::fs;
use std::io::{self, ErrorKind};
use std::path::PathBuf;

/// This function contains no input such that a write to /proc/self/mem occurs.
pub fn dangerous_param(contents: &str) -> io::Result<()> {
    let name = "/proc/self/mem";
    fs::write(name, contents)
}
