use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, atomic::AtomicBool},
};
use zi_devtools::{knowledge_index, knowledge_sources, mcp};

#[test]
fn stdio_search_verifies_live_source_and_never_returns_absolute_path() {
    let executable = std::env::var_os("ZI_DEVTOOLS_MCP_TEST_EXE")
        .map(PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_ZiDevToolsMcp").into());
    let root = std::env::temp_dir().join(format!("zi-knowledge-mcp-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let notes = root.join("notes");
    fs::create_dir(&notes).unwrap();
    let file = notes.join("guide.md");
    fs::write(
        &file,
        "# Local guide\n\nRustMcpProbe confirms this verified local document.",
    )
    .unwrap();
    let mut sources = Vec::new();
    let mut source =
        knowledge_sources::add_source(&mut sources, &notes, "Fixture notes", "").unwrap();
    source.snapshot = Some(knowledge_sources::scan(&source, &AtomicBool::new(false)).unwrap());
    let registry = root.join("sources.json");
    fs::write(
        &registry,
        serde_json::to_vec(
            &json!({"schema":"zi-devtools-knowledge-sources","version":1,"sources":[source]}),
        )
        .unwrap(),
    )
    .unwrap();
    let index = root.join("index.sqlite3");
    let indexed_sources = knowledge_sources::read_sources(&registry).unwrap();
    knowledge_index::sync_all(
        &index,
        &indexed_sources,
        false,
        &AtomicBool::new(false),
        |_, _, _| {},
    )
    .unwrap();

    let client_config = mcp::Config {
        executable: executable.clone(),
        args: vec![
            "--sources".into(),
            registry.display().to_string(),
            "--index".into(),
            index.display().to_string(),
        ],
    };
    let inspected = mcp::run(
        client_config.clone(),
        mcp::Action::Inspect,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(inspected.server, "Zi DevTools Knowledge");
    assert_eq!(inspected.tools.len(), 2);
    let called = mcp::run(
        client_config,
        mcp::Action::Call {
            tool: "search_knowledge".into(),
            arguments: json!({"query":"RustMcpProbe","limit":1}),
            expected_tool: inspected.tools[1].clone(),
        },
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(
        called
            .call_result
            .as_ref()
            .unwrap()
            .pointer("/structuredContent/hits/0/relative_path"),
        Some(&json!("guide.md"))
    );

    let mut child = Command::new(&executable)
        .args([
            "--sources",
            registry.to_str().unwrap(),
            "--index",
            index.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    {
        let mut call = |request: Value| -> Value {
            writeln!(stdin, "{request}").unwrap();
            stdin.flush().unwrap();
            let mut line = String::new();
            stdout.read_line(&mut line).unwrap();
            serde_json::from_str(&line).unwrap()
        };
        let init = call(
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}}}),
        );
        assert_eq!(
            init.pointer("/result/capabilities/tools/listChanged"),
            Some(&json!(false))
        );
        let catalog = call(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}));
        assert_eq!(
            catalog.pointer("/result/tools/1/name"),
            Some(&json!("search_knowledge"))
        );
        let listed = call(
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_knowledge_sources","arguments":{}}}),
        );
        assert_eq!(
            listed.pointer("/result/structuredContent/sources/0/name"),
            Some(&json!("Fixture notes"))
        );
        let source_id = listed
            .pointer("/result/structuredContent/sources/0/id")
            .and_then(Value::as_str)
            .unwrap();
        let found = call(
            json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"search_knowledge","arguments":{"query":"RustMcpProbe","source_id":source_id,"limit":2}}}),
        );
        assert_eq!(
            found.pointer("/result/structuredContent/hits/0/relative_path"),
            Some(&json!("guide.md"))
        );
        assert!(!found.to_string().contains(&root.display().to_string()));
        fs::write(&file, "# Changed\n\nRustMcpProbe is now a different file.").unwrap();
        let stale = call(
            json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"search_knowledge","arguments":{"query":"RustMcpProbe"}}}),
        );
        assert_eq!(
            stale.pointer("/result/structuredContent/hits"),
            Some(&json!([]))
        );
        assert_eq!(
            stale.pointer("/result/structuredContent/stale_skipped"),
            Some(&json!(1))
        );
        let invalid = call(
            json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"search_knowledge","arguments":{"query":"RustMcpProbe","limit":11}}}),
        );
        assert_eq!(invalid.pointer("/result/isError"), Some(&json!(true)));
        fs::write(&registry, "{broken").unwrap();
        let unreadable = call(
            json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"search_knowledge","arguments":{"query":"RustMcpProbe"}}}),
        );
        assert_eq!(unreadable.pointer("/result/isError"), Some(&json!(true)));
        assert!(!unreadable.to_string().contains("verified local document"));
    }
    drop(stdin);
    drop(stdout);
    assert!(child.wait().unwrap().success());
    fs::remove_dir_all(root).unwrap();
}
