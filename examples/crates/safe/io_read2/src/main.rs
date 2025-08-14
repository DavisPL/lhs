use std::io::{Read};
use std::net::{TcpStream};
use std::fs;
use std::path::Path;

fn main() -> std::io::Result<()> {

    let mut stream = TcpStream::connect("127.0.0.1:9500")?;
    let mut filename_buf = [0u8; 64];
    let filename_len = stream.read(&mut filename_buf)?;
    let filename = String::from_utf8_lossy(&filename_buf[..filename_len]).to_string();
    let filename_as_path = Path::new(&filename);
    let storage_file = filename_as_path.join("LHS.txt");

    let mut content = "LHS now supports io::read";
    fs::write(&storage_file, content)?;
    Ok(())
}
