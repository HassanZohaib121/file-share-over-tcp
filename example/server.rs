
use std::net::TcpListener;
use std::io::{Read, Write};

fn main() -> std::io::Result<()> {
    // Create TCP listener
    let listener = TcpListener::bind("127.0.0.1:7878")?;

    println!("TCP Server listening on 127.0.0.1:7878");

    // Accept incoming connections
    for stream in listener.incoming() {
        let mut stream = stream?;

        println!("Client connected!");

        let mut buffer = [0; 1024];

        // Read data from client
        let bytes_read = stream.read(&mut buffer)?;

        let message = String::from_utf8_lossy(&buffer[..bytes_read]);

        println!("Received: {}", message);

        // Send response
        stream.write_all(b"Hello from Rust TCP Server!")?;
    }

    Ok(())
}
