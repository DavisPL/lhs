use std::io::Read;
use std::net::TcpStream;
use std::fs;
use std::path::PathBuf;

fn main() -> std::io::Result<()> {
    let mut stream = TcpStream::connect("127.0.0.1:9500")?;
    let mut filename_buf = [0u8; 64];
    let filename_len = stream.read(&mut filename_buf)?;
    let filename = String::from_utf8_lossy(&filename_buf[..filename_len]).to_string();

    // Create a mutable PathBuf and push onto it
    let mut storage_file = PathBuf::from(&filename);
    storage_file.push("LHS.txt");

    let content = "LHS now supports io::read - EVIL LARRY HAS BEEN DEFEATED";
    fs::write(&storage_file, content)?;
    Ok(())
}
