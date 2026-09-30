use std::sync::atomic::AtomicBool;
use zi_devtools::embedding;

#[test]
#[ignore = "requires local Ollama with bge-m3:latest; sends only synthetic text"]
fn real_bge_m3_returns_two_bounded_vectors() {
    let result = embedding::compare(
        "http://127.0.0.1:11434/api/embed",
        "bge-m3:latest",
        "Rust compiler checks ownership rules.",
        "Rust language uses a borrow checker.",
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(result.dimensions, 1024);
    assert!(result.cosine.is_finite());
    assert!(result.euclidean.is_finite());
    assert_eq!(result.vector_a.len(), result.vector_b.len());
}
