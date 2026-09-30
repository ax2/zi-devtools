//! Optional end-to-end RAG evaluation with a disposable synthetic document.
use std::{fs, sync::atomic::AtomicBool};
use zi_devtools::{
    knowledge_answer::{Protocol, Retrieval},
    knowledge_eval::{self, ModelConfig, RunOptions},
    knowledge_index,
    knowledge_sources::{add_source, scan},
    vector_index,
};

#[test]
#[ignore = "requires local Ollama with qwen2.5:7b and bge-m3:latest"]
fn evaluates_synthetic_retrieval_and_local_model() {
    let root = std::env::temp_dir().join(format!("zi-eval-smoke-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("guide.txt"),
        "项目代号是银杉。银杉项目的准入码是 ZX-42。",
    )
    .unwrap();
    let mut registry = Vec::new();
    let mut source = add_source(&mut registry, &root, "合成文档", "").unwrap();
    source.snapshot = Some(scan(&source, &AtomicBool::new(false)).unwrap());
    let index = root.join("knowledge-index.sqlite3");
    knowledge_index::sync_all(
        &index,
        &[source.clone()],
        false,
        &AtomicBool::new(false),
        |_, _, _| {},
    )
    .unwrap();
    let vectors = root.join("vectors.sqlite3");
    vector_index::sync(
        &vectors,
        &index,
        "http://127.0.0.1:11434/api/embed",
        "bge-m3:latest",
        &AtomicBool::new(false),
        |_, _| {},
    )
    .unwrap();
    let suite = knowledge_eval::parse_suite(
        r#"{"version":1,"cases":[{"id":"code","question":"银杉项目的准入码是什么？","terms":"准入码","expected_source":"合成文档","expected_relative":"guide.txt","expected_answer_contains":"ZX-42"}]}"#,
        &[source.clone()],
    )
    .unwrap();
    let config = ModelConfig {
        endpoint: "http://127.0.0.1:11434/api/chat".into(),
        protocol: Protocol::Ollama,
        model: "qwen2.5:7b".into(),
    };
    let report = knowledge_eval::run(
        &index,
        &suite,
        &[source.clone()],
        3,
        Some(&config),
        &AtomicBool::new(false),
        |_, _, _| {},
    )
    .unwrap();
    assert_eq!(report.retrieval_hits, 1);
    assert_eq!(report.cited_expected, 1);
    assert_eq!(report.literal_answer_matches, 1);
    assert!(report.results[0].error.is_none());
    let hybrid = Retrieval::Hybrid {
        vector_path: vectors,
        endpoint: "http://127.0.0.1:11434/api/embed".into(),
        model: "bge-m3:latest".into(),
        keyword_weight: 50,
    };
    let mixed = knowledge_eval::run_with(
        &index,
        &suite,
        &[source.clone()],
        RunOptions {
            top_k: 3,
            model: Some(&config),
            retrieval: hybrid.clone(),
        },
        &AtomicBool::new(false),
        |_, _, _| {},
    )
    .unwrap();
    assert_eq!(mixed.retrieval, "hybrid");
    assert_eq!((mixed.retrieval_hits, mixed.cited_expected), (1, 1));
    assert!(mixed.results[0].error.is_none());
    let comparison = knowledge_eval::compare(
        &index,
        &suite,
        &[source],
        3,
        hybrid,
        &AtomicBool::new(false),
        |_, _, _| {},
    )
    .unwrap();
    assert_eq!((comparison.paired, comparison.both), (1, 1));
    fs::remove_dir_all(root).unwrap();
}
