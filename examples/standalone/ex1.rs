use std::fs;
use std::io;

pub fn write_to_file(contents: &str, filename: &str) -> io::Result<()> {
    fs::write(filename, contents)?;
    Ok(())
}
