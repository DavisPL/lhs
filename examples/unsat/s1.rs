use std::fs;
use std::io::{self, ErrorKind};
use std::path::PathBuf;

/// This function contains no input such that a write to /proc/self/mem occurs.
pub fn write_to_hw3(contents: &str) -> io::Result<()> {
    fs::write("~/caleb/hw3/src/main.rs", contents)
}
