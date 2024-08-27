use std::fs;
use std::io::{self, ErrorKind};
use std::path::PathBuf;

/// This function contains no input such that a write to /proc/self/mem occurs.
pub fn write_to_file_actually_safe(contents: &str, filename: &str) -> io::Result<()> {
    let filename_realpath: PathBuf = fs::canonicalize(filename)?;
    let unsafe_path: PathBuf = fs::canonicalize("/proc/self/mem")?;
    if filename_realpath == unsafe_path {
        return Err(io::Error::new(ErrorKind::Other, "Unsafe write!"));
    }
    fs::write(filename_realpath, contents)?;
    Ok(())
}
