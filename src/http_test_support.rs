//! Bounded local transport for HTTP fixtures; never used by production clients.
use std::{
    io::{ErrorKind, Read},
    net::{TcpListener, TcpStream},
    thread,
    time::{Duration, Instant},
};

pub(crate) fn accept_get(listener: TcpListener) -> TcpStream {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                assert!(
                    Instant::now() < deadline,
                    "HTTP fixture received no connection"
                );
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("HTTP fixture accept failed: {error}"),
        }
    };
    // Windows may carry the listener's nonblocking mode to accepted sockets.
    // Read timeouts do not switch them back to blocking mode.
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    // A single TCP read need not contain all request headers. Closing with
    // unread incoming bytes can reset the connection before the client sees
    // the intended test response. These fixtures accept bounded GETs only.
    let mut headers = Vec::new();
    while !headers.windows(4).any(|window| window == b"\r\n\r\n") {
        let mut buffer = [0; 1024];
        let count = stream.read(&mut buffer).unwrap();
        assert!(count > 0, "HTTP fixture received incomplete headers");
        headers.extend_from_slice(&buffer[..count]);
        assert!(headers.len() <= 16 * 1024, "HTTP fixture headers too large");
    }
    assert!(
        headers.starts_with(b"GET "),
        "HTTP fixture accepts GET only"
    );
    stream
}
