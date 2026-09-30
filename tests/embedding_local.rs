use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use zi_devtools::embedding;

fn fixture(status: &str, body: Vec<u8>, delay: Duration) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/api/embed", listener.local_addr().unwrap());
    let status = status.to_owned();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0u8; 1024];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let count = stream.read(&mut buffer).unwrap();
            assert!(count > 0 && request.len() < 64 * 1024);
            request.extend_from_slice(&buffer[..count]);
        }
        let headers = String::from_utf8_lossy(&request);
        assert!(headers.starts_with("POST /api/embed HTTP/1.1"));
        let expected = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length: ")
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .unwrap();
        let header_end = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap()
            + 4;
        while request.len() - header_end < expected {
            let count = stream.read(&mut buffer).unwrap();
            assert!(count > 0);
            request.extend_from_slice(&buffer[..count]);
        }
        let posted: serde_json::Value = serde_json::from_slice(&request[header_end..]).unwrap();
        assert_eq!(posted["model"], "fixture-embed");
        assert_eq!(posted["input"], serde_json::json!(["first", "second"]));
        thread::sleep(delay);
        let _ = write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(&body);
    });
    (endpoint, handle)
}

#[test]
fn compares_local_vectors_and_rejects_bad_response() {
    let (endpoint, server) = fixture(
        "200 OK",
        br#"{"model":"fixture-embed","embeddings":[[1,0,0],[0,1,0]]}"#.to_vec(),
        Duration::ZERO,
    );
    let result = embedding::compare(
        &endpoint,
        "fixture-embed",
        "first",
        "second",
        &AtomicBool::new(false),
    )
    .unwrap();
    server.join().unwrap();
    assert_eq!(result.dimensions, 3);
    assert_eq!(result.cosine, 0.0);
    assert!((result.euclidean - 2_f64.sqrt()).abs() < 1e-12);

    let (endpoint, server) = fixture(
        "200 OK",
        br#"{"embeddings":[[1,0],[1]]}"#.to_vec(),
        Duration::ZERO,
    );
    assert!(
        embedding::compare(
            &endpoint,
            "fixture-embed",
            "first",
            "second",
            &AtomicBool::new(false)
        )
        .unwrap_err()
        .to_string()
        .contains("维度不同")
    );
    server.join().unwrap();
}

#[test]
fn cancellation_closes_pending_request_promptly() {
    let (endpoint, server) = fixture(
        "200 OK",
        br#"{"embeddings":[[1],[1]]}"#.to_vec(),
        Duration::from_millis(700),
    );
    let cancelled = Arc::new(AtomicBool::new(false));
    let signal = Arc::clone(&cancelled);
    let started = Instant::now();
    let worker = thread::spawn(move || {
        embedding::compare(&endpoint, "fixture-embed", "first", "second", &signal)
            .unwrap_err()
            .to_string()
    });
    thread::sleep(Duration::from_millis(200));
    cancelled.store(true, Ordering::Relaxed);
    assert!(worker.join().unwrap().contains("已取消"));
    assert!(started.elapsed() < Duration::from_millis(600));
    server.join().unwrap();
}

#[test]
fn rejects_non_success_and_oversized_body() {
    for (status, body, expected) in [
        ("404 Not Found", b"missing".to_vec(), "HTTP 404"),
        ("200 OK", vec![b'x'; 4 * 1024 * 1024 + 1], "4 MiB"),
    ] {
        let (endpoint, server) = fixture(status, body, Duration::ZERO);
        let error = embedding::compare(
            &endpoint,
            "fixture-embed",
            "first",
            "second",
            &AtomicBool::new(false),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains(expected), "{error}");
        server.join().unwrap();
    }
}
