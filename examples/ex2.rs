use std::fs;
use std::io::{self, ErrorKind};

pub fn write_to_file_safe(contents: &str, filename: &str) -> io::Result<()> {
    if filename == "/proc/self/mem" {
        return Err(io::Error::new(ErrorKind::Other, "Unsafe write!"));
    }
    fs::write(filename, contents)?;
    Ok(())
}
