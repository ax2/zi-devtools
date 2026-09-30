//! Optional local-model smoke test with a disposable synthetic document.
use std::{fs, sync::atomic::AtomicBool};
use zi_devtools::{
    knowledge_answer::{self, Protocol},
    knowledge_index,
    knowledge_sources::{add_source, scan},
};

#[test]
#[ignore = "requires a running local Ollama server with qwen2.5:7b"]
fn local_ollama_answers_synthetic_document_with_a_real_citation() {
    let root = std::env::temp_dir().join(format!("zi-answer-smoke-{}", uuid::Uuid::new_v4()));
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
    let prepared = knowledge_answer::prepare(
        &index,
        &[source.clone()],
        "银杉项目的准入码是什么？",
        "准入码",
        None,
        3,
    )
    .unwrap();
    assert_eq!(prepared.evidence.len(), 1);
    let answer = knowledge_answer::generate(
        &prepared,
        &[source],
        "http://127.0.0.1:11434/api/chat",
        Protocol::Ollama,
        "qwen2.5:7b",
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(!answer.insufficient);
    assert_eq!(answer.citations, vec![1]);
    assert!(answer.text.contains("ZX-42"));
    fs::remove_dir_all(root).unwrap();
}
