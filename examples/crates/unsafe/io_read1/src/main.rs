use std::io::Read;
use std::net::TcpStream;
use std::fs;

fn main() -> std::io::Result<()> {
    let mut stream = TcpStream::connect("127.0.0.1:9000")?;

    // First, read the filename
    let mut filename_buf = [0u8; 64];
    let filename_len = stream.read(&mut filename_buf)?;
    let filename = String::from_utf8_lossy(&filename_buf[..filename_len]).to_string();

    // Then, read the file contents
    let mut content_buf = [0u8; 1024];
    let content_len = stream.read(&mut content_buf)?;

    fs::write(&filename, &content_buf[..content_len])?;

    Ok(())
}
