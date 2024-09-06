use std::fs;
use std::io::{self, ErrorKind};
use std::path::PathBuf;

pub fn write_to_hw3(contents: &str) -> io::Result<()> {
    Ok(fs::write(
        "~/UCDavis/PL/2024/summer/REU/LHS/main.rs",
        contents,
    )?)
}
