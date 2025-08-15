use std::io::{self, Read};
use std::net::TcpStream;
use byteorder::{BigEndian, ReadBytesExt};
use std::path::Path;
use std::fs;

fn main() -> io::Result<()> {
    let mut stream = TcpStream::connect("127.0.0.1:8080")?;
    // 4-byte big-endian length header
    let len = stream.read_u32::<BigEndian>()? as usize;

    // Read the exact payload length
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?; // blocks until len bytes or errors 

    // Interpret payload as a UTF-8 path string
    let path_str = String::from_utf8(buf).unwrap_or_default();

    let filename = Path::new(&path_str);

    fs::write(filename, "I AM DANGEROUS")?;

    Ok(())
}
