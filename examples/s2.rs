use std::fs;
use std::io::{self, ErrorKind};
use std::path::PathBuf;

/// This function does contain an input (`filename: /proc/self/mem`) such that a safety property (write to /proc/self/mem) is violated.
pub fn write_to_file(contents: &str, filename: &str) -> io::Result<()> {
    fs::write(contents, filename)?;
    Ok(())
}
