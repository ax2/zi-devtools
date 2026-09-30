//! Explicit end-to-end local-model smoke with disposable synthetic documents.
use std::{fs, sync::atomic::AtomicBool};
use zi_devtools::{knowledge_answer, knowledge_index, knowledge_sources, vector_index};

#[test]
#[ignore = "requires local Ollama with bge-m3:latest and qwen2.5:7b; uses only disposable synthetic text"]
fn hybrid_evidence_reaches_local_answer_with_a_valid_citation() {
    let root = std::env::temp_dir().join(format!("zi-hybrid-answer-{}", uuid::Uuid::new_v4()));
    let documents = root.join("docs");
    fs::create_dir_all(&documents).unwrap();
    fs::write(
        documents.join("guide.txt"),
        "银杉项目的准入码是 ZX-42。该准入码只用于这份合成测试文档。",
    )
    .unwrap();
    fs::write(documents.join("garden.txt"), "花园植物需要阳光和水。").unwrap();
    let mut registry = Vec::new();
    let mut source =
        knowledge_sources::add_source(&mut registry, &documents, "合成文档", "").unwrap();
    source.snapshot = Some(knowledge_sources::scan(&source, &AtomicBool::new(false)).unwrap());
    let keyword = root.join("keyword.sqlite3");
    let vectors = root.join("vectors.sqlite3");
    knowledge_index::sync_all(
        &keyword,
        &[source.clone()],
        false,
        &AtomicBool::new(false),
        |_, _, _| {},
    )
    .unwrap();
    vector_index::sync(
        &vectors,
        &keyword,
        "http://127.0.0.1:11434/api/embed",
        "bge-m3:latest",
        &AtomicBool::new(false),
        |_, _| {},
    )
    .unwrap();
    let prepared = knowledge_answer::prepare_with(
        &keyword,
        &[source.clone()],
        knowledge_answer::PrepareRequest {
            question: "银杉项目的准入码是什么？",
            terms: "银杉项目准入码",
            source_id: None,
            top_k: 3,
            retrieval: knowledge_answer::Retrieval::Hybrid {
                vector_path: vectors,
                endpoint: "http://127.0.0.1:11434/api/embed".into(),
                model: "bge-m3:latest".into(),
                keyword_weight: 50,
            },
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(!prepared.evidence.is_empty());
    assert_eq!(prepared.evidence[0].hit.relative, "guide.txt");
    assert!(prepared.evidence[0].hybrid_trace.is_some());
    let answer = knowledge_answer::generate(
        &prepared,
        &[source],
        "http://127.0.0.1:11434/api/chat",
        knowledge_answer::Protocol::Ollama,
        "qwen2.5:7b",
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(!answer.insufficient);
    assert!(answer.citations.contains(&1));
    assert!(answer.text.contains("ZX-42"));
    fs::remove_dir_all(root).unwrap();
}
