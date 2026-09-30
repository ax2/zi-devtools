use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use zi_devtools::{
    hybrid_search, knowledge_answer, knowledge_index, knowledge_sources, vector_index,
};

struct ModelFixture {
    endpoint: String,
    digest: Arc<Mutex<String>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ModelFixture {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/api/embed", listener.local_addr().unwrap());
        let digest = Arc::new(Mutex::new("a".repeat(64)));
        let stop = Arc::new(AtomicBool::new(false));
        let (server_digest, server_stop) = (Arc::clone(&digest), Arc::clone(&stop));
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
                    let payload: serde_json::Value =
                        serde_json::from_slice(&request[header_end..]).unwrap();
                    assert_eq!(payload["model"], "fixture-embed:latest");
                    let vectors = payload["input"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|value| {
                            let text = value.as_str().unwrap().to_ascii_lowercase();
                            if text.contains("rust") || text.contains("ownership") {
                                vec![1.0, 0.0]
                            } else {
                                vec![0.0, 1.0]
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
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for ModelFixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

#[test]
fn hybrid_uses_both_channels_filters_sources_and_rejects_changed_model() {
    let root = std::env::temp_dir().join(format!("zi-hybrid-test-{}", uuid::Uuid::new_v4()));
    let primary = root.join("primary");
    let secondary = root.join("secondary");
    fs::create_dir_all(&primary).unwrap();
    fs::create_dir_all(&secondary).unwrap();
    fs::write(
        primary.join("rust.txt"),
        "Rust ownership avoids data races.",
    )
    .unwrap();
    fs::write(
        primary.join("solar.txt"),
        "Solar panels generate electricity.",
    )
    .unwrap();
    fs::write(
        secondary.join("other.txt"),
        "Rust ownership is checked by the compiler.",
    )
    .unwrap();
    let mut registry = Vec::new();
    let mut first = knowledge_sources::add_source(&mut registry, &primary, "Primary", "").unwrap();
    let mut second =
        knowledge_sources::add_source(&mut registry, &secondary, "Secondary", "").unwrap();
    first.snapshot = Some(knowledge_sources::scan(&first, &AtomicBool::new(false)).unwrap());
    second.snapshot = Some(knowledge_sources::scan(&second, &AtomicBool::new(false)).unwrap());
    let keyword = root.join("keyword.sqlite3");
    let vectors = root.join("vectors.sqlite3");
    knowledge_index::sync_all(
        &keyword,
        &[first.clone(), second.clone()],
        false,
        &AtomicBool::new(false),
        |_, _, _| {},
    )
    .unwrap();
    let server = ModelFixture::new();
    let model = "fixture-embed:latest";
    vector_index::sync(
        &vectors,
        &keyword,
        &server.endpoint,
        model,
        &AtomicBool::new(false),
        |_, _| {},
    )
    .unwrap();
    let both = hybrid_search::search(
        &keyword,
        &vectors,
        &server.endpoint,
        model,
        "Rust ownership",
        hybrid_search::SearchOptions {
            source_id: None,
            limit: 10,
            keyword_weight: 50,
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(both.keyword_candidates, 2);
    assert_eq!(both.semantic_candidates, 3);
    assert_eq!(both.hits.len(), 3);
    assert!(
        both.hits
            .iter()
            .any(|item| item.keyword_rank.is_some() && item.semantic_rank.is_some())
    );
    assert!(
        both.hits
            .iter()
            .any(|item| item.keyword_rank.is_none() && item.semantic_rank.is_some())
    );
    let filtered = hybrid_search::search(
        &keyword,
        &vectors,
        &server.endpoint,
        model,
        "Rust ownership",
        hybrid_search::SearchOptions {
            source_id: Some(&second.id),
            limit: 10,
            keyword_weight: 50,
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(filtered.hits.len(), 1);
    assert!(
        filtered
            .hits
            .iter()
            .all(|item| item.hit.source_id == second.id)
    );
    let prepared = knowledge_answer::prepare_with(
        &keyword,
        &[first.clone(), second.clone()],
        knowledge_answer::PrepareRequest {
            question: "How does Rust ownership work?",
            terms: "Rust ownership",
            source_id: None,
            top_k: 3,
            retrieval: knowledge_answer::Retrieval::Hybrid {
                vector_path: vectors.clone(),
                endpoint: server.endpoint.clone(),
                model: model.into(),
                keyword_weight: 50,
            },
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(prepared.evidence.len(), 3);
    assert_eq!(prepared.semantic_candidates, 3);
    assert!(prepared.evidence.iter().any(|item| {
        item.hybrid_trace
            .as_ref()
            .is_some_and(|trace| trace.keyword_rank.is_none() && trace.semantic_rank.is_some())
    }));
    assert_eq!(prepared.evidence[0].id, 1);
    assert!(knowledge_answer::messages(&prepared).is_ok());
    let prepared_filtered = knowledge_answer::prepare_with(
        &keyword,
        &[first.clone(), second.clone()],
        knowledge_answer::PrepareRequest {
            question: "How does Rust ownership work?",
            terms: "Rust ownership",
            source_id: Some(&second.id),
            top_k: 3,
            retrieval: knowledge_answer::Retrieval::Hybrid {
                vector_path: vectors.clone(),
                endpoint: server.endpoint.clone(),
                model: model.into(),
                keyword_weight: 75,
            },
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(prepared_filtered.evidence.len(), 1);
    assert_eq!(prepared_filtered.evidence[0].hit.source_id, second.id);

    let old_hit = both
        .hits
        .iter()
        .find(|item| item.hit.relative == "rust.txt")
        .unwrap()
        .hit
        .clone();
    fs::write(
        primary.join("rust.txt"),
        "Updated Rust ownership avoids races.",
    )
    .unwrap();
    let stale_answer = knowledge_answer::generate(
        &prepared,
        &[first.clone(), second.clone()],
        "http://127.0.0.1:9/api/chat",
        knowledge_answer::Protocol::Ollama,
        "fixture",
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(format!("{stale_answer:#}").contains("预览后变化"));
    first.snapshot = Some(knowledge_sources::scan(&first, &AtomicBool::new(false)).unwrap());
    knowledge_index::sync_all(
        &keyword,
        &[first, second],
        false,
        &AtomicBool::new(false),
        |_, _, _| {},
    )
    .unwrap();
    let (_, stale) = knowledge_index::filter_current_hits(&keyword, &[old_hit]).unwrap();
    assert_eq!(stale, 1);
    let changed = hybrid_search::search(
        &keyword,
        &vectors,
        &server.endpoint,
        model,
        "Rust ownership",
        hybrid_search::SearchOptions {
            source_id: None,
            limit: 10,
            keyword_weight: 50,
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    let revised = changed
        .hits
        .iter()
        .find(|item| item.hit.relative == "rust.txt")
        .unwrap();
    assert!(revised.keyword_rank.is_some());
    assert!(revised.semantic_rank.is_none());
    *server.digest.lock().unwrap() = "b".repeat(64);
    let stale_model = knowledge_answer::prepare_with(
        &keyword,
        &[],
        knowledge_answer::PrepareRequest {
            question: "How does Rust ownership work?",
            terms: "Rust ownership",
            source_id: None,
            top_k: 3,
            retrieval: knowledge_answer::Retrieval::Hybrid {
                vector_path: vectors.clone(),
                endpoint: server.endpoint.clone(),
                model: model.into(),
                keyword_weight: 50,
            },
        },
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(stale_model.to_string().contains("版本已变化"));
    assert!(
        hybrid_search::search(
            &keyword,
            &vectors,
            &server.endpoint,
            model,
            "Rust ownership",
            hybrid_search::SearchOptions {
                source_id: None,
                limit: 10,
                keyword_weight: 50,
            },
            &AtomicBool::new(false)
        )
        .unwrap_err()
        .to_string()
        .contains("版本已变化")
    );
    let cancelled = AtomicBool::new(true);
    assert!(
        hybrid_search::search(
            &keyword,
            &vectors,
            &server.endpoint,
            model,
            "Rust ownership",
            hybrid_search::SearchOptions {
                source_id: None,
                limit: 10,
                keyword_weight: 50,
            },
            &cancelled
        )
        .unwrap_err()
        .to_string()
        .contains("已取消")
    );
    drop(server);
    fs::remove_dir_all(root).unwrap();
}
