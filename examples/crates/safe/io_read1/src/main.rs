use std::io::Read;
use std::net::TcpStream;
use std::fs;

fn main() -> std::io::Result<()> {
    let mut stream = TcpStream::connect("127.0.0.1:9500")?;
    let mut filename_buf = [0u8; 64];
    let filename_len = stream.read(&mut filename_buf)?;
    let filename = String::from_utf8_lossy(&filename_buf[..filename_len]).to_string();

    // Join path and file name
    let updated_filename = format!("{}/message.txt", filename);

    let content = "Dark Lord has been defeated";
    fs::write(&updated_filename, content)?;
    Ok(())
}
