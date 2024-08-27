use std::fs;
use std::io::{self, ErrorKind};
use std::path::PathBuf;

/// alternative example with pure if/else
pub fn write_to_file_safe2(contents: &str, filename: &str) -> io::Result<()> {
    if filename == "/proc/self/mem" {
        Err(io::Error::new(ErrorKind::Other, "Unsafe write!"))
    } else {
        fs::write(filename, contents)
    }
}
