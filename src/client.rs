
use std::net::TcpStream;
use std::io::{Read, Write};

fn main() -> std::io::Result<()> {
    // Connect to TCP server
    let mut stream = TcpStream::connect("127.0.0.1:7878")?;

    println!("Connected to TCP Server!");

    // Send message
    stream.write_all(b"Hello from Rust TCP Client!")?;

    // Read server response
    let mut buffer = [0; 1024];

    let bytes_read = stream.read(&mut buffer)?;

    let response = String::from_utf8_lossy(&buffer[..bytes_read]);

    println!("Server response: {}", response);

    Ok(())
}
