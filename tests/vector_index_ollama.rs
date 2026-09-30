use std::{fs, sync::atomic::AtomicBool};
use zi_devtools::{hybrid_search, knowledge_index, knowledge_sources, vector_index};

#[test]
#[ignore = "requires local Ollama with bge-m3:latest; uses only disposable synthetic documents"]
fn real_bge_m3_indexes_and_retrieves_disposable_documents() {
    let root = std::env::temp_dir().join(format!("zi-real-vector-{}", uuid::Uuid::new_v4()));
    let documents = root.join("docs");
    fs::create_dir_all(&documents).unwrap();
    fs::write(
        documents.join("rust.txt"),
        "Rust ownership and borrowing prevent data races in concurrent programs.",
    )
    .unwrap();
    fs::write(
        documents.join("garden.txt"),
        "Garden plants need sunlight, water, and healthy soil.",
    )
    .unwrap();
    let mut sources = Vec::new();
    let mut source =
        knowledge_sources::add_source(&mut sources, &documents, "Synthetic", "").unwrap();
    source.snapshot = Some(knowledge_sources::scan(&source, &AtomicBool::new(false)).unwrap());
    let keyword = root.join("keyword.sqlite3");
    let vectors = root.join("vectors.sqlite3");
    knowledge_index::sync_all(
        &keyword,
        &[source],
        false,
        &AtomicBool::new(false),
        |_, _, _| {},
    )
    .unwrap();
    let endpoint = "http://127.0.0.1:11434/api/embed";
    let model = "bge-m3:latest";
    let report = vector_index::sync(
        &vectors,
        &keyword,
        endpoint,
        model,
        &AtomicBool::new(false),
        |_, _| {},
    )
    .unwrap();
    assert_eq!((report.added, report.dimensions), (2, 1024));
    let hits = vector_index::search(
        &vectors,
        &keyword,
        endpoint,
        model,
        "How does the Rust language prevent concurrency data races?",
        2,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].hit.relative, "rust.txt");
    assert!(hits[0].cosine > hits[1].cosine);
    let hybrid = hybrid_search::search(
        &keyword,
        &vectors,
        endpoint,
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
    assert_eq!(hybrid.hits[0].hit.relative, "rust.txt");
    assert!(hybrid.hits[0].keyword_rank.is_some());
    assert!(hybrid.hits[0].semantic_rank.is_some());
    fs::remove_dir_all(root).unwrap();
}
