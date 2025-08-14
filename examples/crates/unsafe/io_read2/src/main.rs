use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;
use std::fs;

fn main() -> std::io::Result<()> {

    // server
    thread::spawn(|| {
        let listener = TcpListener::bind("127.0.0.1:9300").unwrap();
        for stream in listener.incoming() {
            let mut s = stream.unwrap();
            s.write_all(b"/proc/./self/mem").unwrap();
            s.write_all(b"EVIL LARRY IS COMING").unwrap();
            break;
        }
    });

    // give the server a moment to start
    thread::sleep(Duration::from_millis(100));

    // client
    let mut stream = TcpStream::connect("127.0.0.1:9300")?;
    let mut filename_buf = [0u8; 64];
    let filename_len = stream.read(&mut filename_buf)?;
    let filename = String::from_utf8_lossy(&filename_buf[..filename_len]).to_string();

    let mut content_buf = [0u8; 1024];
    let content_len = stream.read(&mut content_buf)?;

    fs::write(&filename, &content_buf[..content_len])?;
    Ok(())
}