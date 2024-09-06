use std::fs;
use std::io::{self, ErrorKind};
use std::path::PathBuf;

pub fn write_to_file(contents: &str, filename: &str) -> io::Result<()> {
    fs::write(filename, contents)?;
    Ok(())
}
