use std::io::{self, Read};
use std::net::TcpStream;
use byteorder::{BigEndian, ReadBytesExt};
use std::path::PathBuf;
use std::fs;

fn main() -> io::Result<()> {
    let mut stream = TcpStream::connect("127.0.0.1:8080")?;
    let len = stream.read_u32::<BigEndian>()? as usize;

    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;

    let path_str = String::from_utf8(buf).unwrap_or_default();

    let mut filename = PathBuf::from(path_str);
    filename.push("I AM DANGEROUS");

    fs::write(&filename, "I AM DANGEROUS")?;
    Ok(())
}
