use std::fs;
use std::io::{self, ErrorKind};
use std::path::PathBuf;

/// This function also contains an input (filename: `/proc/../proc/self/mem`) such that a safety property (write to /proc/self/mem) is violated.
pub fn write_to_file_safe(contents: &str, filename: &str) -> io::Result<()> {
    if filename == "/proc/self/mem" {
        return Err(io::Error::new(ErrorKind::Other, "Unsafe write!"));
    }
    fs::write(filename, contents)?;
    Ok(())
}
