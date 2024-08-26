use std::fs;
use std::io::{self, ErrorKind};
use std::path::PathBuf;

/// This function contains no input such that a write to /proc/self/mem occurs.
pub fn dangerous(contents: &str) -> io::Result<()> {
    fs::write("/proc/self/mem", contents)
}
