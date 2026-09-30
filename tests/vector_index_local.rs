use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};
use zi_devtools::{knowledge_index, knowledge_sources, vector_index};

struct OllamaFixture {
    endpoint: String,
    digest: Arc<Mutex<String>>,
    embeds: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl OllamaFixture {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/api/embed", listener.local_addr().unwrap());
        let digest = Arc::new(Mutex::new("a".repeat(64)));
        let embeds = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (server_digest, server_embeds, server_stop) =
            (Arc::clone(&digest), Arc::clone(&embeds), Arc::clone(&stop));
        let worker = thread::spawn(move || {
            while !server_stop.load(Ordering::Relaxed) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0u8; 4096];
                let header_end = loop {
                    let count = stream.read(&mut buffer).unwrap();
                    assert!(count > 0 && request.len() < 128 * 1024);
                    request.extend_from_slice(&buffer[..count]);
                    if let Some(pos) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                        break pos + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let body = if headers.starts_with("POST /api/embed ") {
                    let size = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .and_then(|value| value.trim().parse::<usize>().ok())
                        })
                        .unwrap();
                    while request.len() - header_end < size {
                        let count = stream.read(&mut buffer).unwrap();
                        assert!(count > 0);
                        request.extend_from_slice(&buffer[..count]);
                    }
                    server_embeds.fetch_add(1, Ordering::Relaxed);
                    let payload: serde_json::Value =
                        serde_json::from_slice(&request[header_end..]).unwrap();
                    assert_eq!(payload["model"], "fixture-embed:latest");
                    let vectors = payload["input"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|value| {
                            let text = value.as_str().unwrap().to_ascii_lowercase();
                            if text.contains("cat") || text.contains("feline") {
                                vec![1.0, 0.0]
                            } else if text.contains("dog") {
                                vec![0.0, 1.0]
                            } else {
                                vec![0.5, 0.5]
                            }
                        })
                        .collect::<Vec<_>>();
                    serde_json::to_vec(&serde_json::json!({"embeddings":vectors})).unwrap()
                } else {
                    assert!(headers.starts_with("GET /api/tags "), "{headers}");
                    serde_json::to_vec(&serde_json::json!({"models":[{"name":"fixture-embed:latest","digest":server_digest.lock().unwrap().clone()}]})).unwrap()
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(&body);
            }
        });
        Self {
            endpoint,
            digest,
            embeds,
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for OllamaFixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

#[test]
fn incremental_vectors_reject_stale_model_and_rollback_cancellation() {
    let root = std::env::temp_dir().join(format!("zi-vector-test-{}", uuid::Uuid::new_v4()));
    let documents = root.join("docs");
    fs::create_dir_all(&documents).unwrap();
    fs::write(documents.join("cats.txt"), "Cats are small feline animals.").unwrap();
    fs::write(documents.join("dogs.txt"), "Dogs are loyal animals.").unwrap();
    let keyword = root.join("keyword.sqlite3");
    let vectors = root.join("vectors.sqlite3");
    let mut sources = Vec::new();
    let mut source =
        knowledge_sources::add_source(&mut sources, &documents, "Fixture", "").unwrap();
    source.snapshot = Some(knowledge_sources::scan(&source, &AtomicBool::new(false)).unwrap());
    knowledge_index::sync_all(
        &keyword,
        &[source.clone()],
        false,
        &AtomicBool::new(false),
        |_, _, _| {},
    )
    .unwrap();
    let server = OllamaFixture::new();
    let model = "fixture-embed:latest";
    let first = vector_index::sync(
        &vectors,
        &keyword,
        &server.endpoint,
        model,
        &AtomicBool::new(false),
        |_, _| {},
    )
    .unwrap();
    assert_eq!(
        (
            first.added,
            first.updated,
            first.removed,
            first.unchanged,
            first.dimensions
        ),
        (2, 0, 0, 0, 2)
    );
    let hits = vector_index::search(
        &vectors,
        &keyword,
        &server.endpoint,
        model,
        "feline",
        10,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(hits[0].hit.relative.ends_with("cats.txt"));
    assert!(hits[0].cosine > hits[1].cosine);
    let count = server.embeds.load(Ordering::Relaxed);
    let again = vector_index::sync(
        &vectors,
        &keyword,
        &server.endpoint,
        model,
        &AtomicBool::new(false),
        |_, _| {},
    )
    .unwrap();
    assert_eq!(
        (again.added, again.updated, again.removed, again.unchanged),
        (0, 0, 0, 2)
    );
    assert_eq!(server.embeds.load(Ordering::Relaxed), count);

    fs::write(
        documents.join("cats.txt"),
        "Cats and felines have whiskers.",
    )
    .unwrap();
    fs::remove_file(documents.join("dogs.txt")).unwrap();
    source.snapshot = Some(knowledge_sources::scan(&source, &AtomicBool::new(false)).unwrap());
    knowledge_index::sync_all(
        &keyword,
        &[source],
        false,
        &AtomicBool::new(false),
        |_, _, _| {},
    )
    .unwrap();
    let signal = AtomicBool::new(false);
    let cancelled = vector_index::sync(
        &vectors,
        &keyword,
        &server.endpoint,
        model,
        &signal,
        |_, _| signal.store(true, Ordering::Relaxed),
    )
    .unwrap_err();
    assert!(cancelled.to_string().contains("已取消"));
    let conn = rusqlite::Connection::open(&vectors).unwrap();
    let old_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM vectors", [], |row| row.get(0))
        .unwrap();
    assert_eq!(old_count, 2);
    drop(conn);
    let changed = vector_index::sync(
        &vectors,
        &keyword,
        &server.endpoint,
        model,
        &AtomicBool::new(false),
        |_, _| {},
    )
    .unwrap();
    assert_eq!(
        (
            changed.added,
            changed.updated,
            changed.removed,
            changed.unchanged
        ),
        (0, 1, 1, 0)
    );
    *server.digest.lock().unwrap() = "b".repeat(64);
    assert!(
        vector_index::search(
            &vectors,
            &keyword,
            &server.endpoint,
            model,
            "cats",
            10,
            &AtomicBool::new(false)
        )
        .unwrap_err()
        .to_string()
        .contains("版本已变化")
    );
    let rebuilt = vector_index::sync(
        &vectors,
        &keyword,
        &server.endpoint,
        model,
        &AtomicBool::new(false),
        |_, _| {},
    )
    .unwrap();
    assert_eq!(
        (
            rebuilt.added,
            rebuilt.removed,
            rebuilt.unchanged,
            rebuilt.dimensions
        ),
        (1, 1, 0, 2)
    );
    knowledge_index::sync_all(&keyword, &[], false, &AtomicBool::new(false), |_, _, _| {}).unwrap();
    let removed = vector_index::sync(
        &vectors,
        &keyword,
        &server.endpoint,
        model,
        &AtomicBool::new(false),
        |_, _| {},
    )
    .unwrap();
    assert_eq!(removed.removed, 1);
    assert!(
        vector_index::search(
            &vectors,
            &keyword,
            &server.endpoint,
            model,
            "cats",
            10,
            &AtomicBool::new(false)
        )
        .unwrap()
        .is_empty()
    );
    vector_index::delete_index(&vectors).unwrap();
    assert!(!vectors.exists());
    assert!(keyword.exists());
    assert!(documents.join("cats.txt").exists());
    drop(server);
    fs::remove_dir_all(root).unwrap();
}
