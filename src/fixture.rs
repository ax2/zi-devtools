use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
};

use anyhow::Result;

pub fn run(port: u16) -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    println!("fixture listening on http://127.0.0.1:{port}");
    std::io::stdout().flush()?;
    for stream in listener.incoming() {
        let mut stream = stream?;
        let mut request = [0_u8; 2048];
        let size = stream.read(&mut request)?;
        if size == 0 {
            continue;
        }
        let first_line = String::from_utf8_lossy(&request[..size])
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned();
        println!("request: {first_line}");
        std::io::stdout().flush()?;
        let body = "{\"status\":\"ok\",\"source\":\"zi-devtools-fixture\"}";
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        if first_line.starts_with("POST /shutdown ") {
            println!("fixture graceful shutdown");
            std::io::stdout().flush()?;
            break;
        }
    }
    Ok(())
}

pub fn shutdown(port: u16) -> Result<()> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.write_all(b"POST /shutdown HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?;
    let mut response = [0_u8; 256];
    let _ = stream.read(&mut response)?;
    Ok(())
}
