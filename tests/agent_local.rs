use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use zi_devtools::{
    agent,
    mcp::{self, Action},
    mcp_access::{self, Rule, Store},
};

fn python() -> PathBuf {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .flat_map(|directory| {
            ["python.exe", "python"]
                .into_iter()
                .map(move |name| directory.join(name))
        })
        .find(|path| path.is_absolute() && path.is_file())
        .expect("Python is required for the disposable MCP fixture")
}

fn server() -> mcp::Config {
    mcp::Config {
        executable: python(),
        args: vec![
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/mcp_stdio_server.py")
                .to_string_lossy()
                .into_owned(),
            "normal".into(),
        ],
    }
}

fn knowledge_server() -> mcp::Config {
    let mut config = server();
    *config.args.last_mut().unwrap() = "knowledge".into();
    config
}

fn token() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

fn approved() -> (PathBuf, PathBuf, Value) {
    let root = std::env::temp_dir().join(format!("zi-agent-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let access_path = root.join("permissions.json");
    let server = server();
    let scope = mcp_access::server_scope(&server).unwrap();
    let report = mcp::run(server, Action::Inspect, token()).unwrap();
    let tool = report
        .tools
        .iter()
        .find(|item| item["name"] == "echo")
        .unwrap()
        .clone();
    let mut store = Store::load(access_path.clone());
    store
        .set(&scope, &tool, Some(Rule::AllowDeclaredReadOnly))
        .unwrap();
    (root, access_path, tool)
}

fn model_server(responses: Vec<Value>) -> (String, std::thread::JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}/api/chat", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for response in responses {
            let deadline = Instant::now() + Duration::from_secs(8);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("model fixture accept failed: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = Vec::new();
            let header_end = loop {
                let mut chunk = [0u8; 4096];
                let count = stream.read(&mut chunk).unwrap();
                assert!(count > 0, "model request closed early");
                bytes.extend_from_slice(&chunk[..count]);
                assert!(bytes.len() <= 128 * 1024);
                if let Some(at) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    break at + 4;
                }
            };
            let headers = String::from_utf8_lossy(&bytes[..header_end]);
            assert!(headers.starts_with("POST /api/chat HTTP/1.1"));
            let length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|value| value.trim().parse::<usize>().ok())
                })
                .unwrap();
            while bytes.len() - header_end < length {
                let mut chunk = [0u8; 4096];
                let count = stream.read(&mut chunk).unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&chunk[..count]);
            }
            requests.push(serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap());
            let body = serde_json::to_vec(&response).unwrap();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
            stream.write_all(&body).unwrap();
            stream.flush().unwrap();
        }
        requests
    });
    (endpoint, handle)
}

fn config(endpoint: String, access_path: PathBuf, max_calls: usize) -> agent::Config {
    agent::Config {
        endpoint,
        model: "fixture-model".into(),
        server: server(),
        access_path,
        goal: "Echo a synthetic phrase and summarize it".into(),
        selected: vec!["echo".into()],
        max_calls,
    }
}

fn plan_reply() -> Value {
    json!({"message":{"role":"assistant","content":"1. Call echo. 2. Summarize the returned phrase."},"eval_count":20})
}
fn call_reply(name: &str) -> Value {
    json!({"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":name,"arguments":{"text":"synthetic evidence"}}}]},"eval_count":25})
}
fn answer_reply() -> Value {
    json!({"message":{"role":"assistant","content":"The fixture returned synthetic evidence."},"eval_count":30})
}

#[test]
fn approved_plan_executes_only_the_selected_read_only_tool() {
    let (root, access_path, _) = approved();
    let (endpoint, fixture) = model_server(vec![plan_reply(), call_reply("echo"), answer_reply()]);
    let inspection = agent::inspect(&server(), &access_path, token()).unwrap();
    assert_eq!(inspection.tools.len(), 1);
    assert_eq!(inspection.tools[0]["name"], "echo");
    let plan = agent::prepare(config(endpoint, access_path, 2), token()).unwrap();
    assert!(plan.text.contains("Call echo"));
    let mut streamed = Vec::new();
    let outcome = agent::execute(plan, token(), |step| streamed.push(step)).unwrap();
    assert_eq!(outcome.steps.len(), 1);
    assert_eq!(streamed[0].tool, "echo");
    assert!(streamed[0].result.contains("内容项"));
    assert!(outcome.answer.contains("synthetic evidence"));
    let requests = fixture.join().unwrap();
    assert!(requests[0]["tools"].as_array().unwrap().is_empty());
    assert_eq!(requests[1]["tools"][0]["function"]["name"], "echo");
    assert_eq!(
        requests[2]["messages"].as_array().unwrap().last().unwrap()["role"],
        "tool"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn knowledge_result_references_survive_the_agent_step_without_excerpt() {
    let root = std::env::temp_dir().join(format!("zi-agent-evidence-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let access_path = root.join("permissions.json");
    let server = knowledge_server();
    let scope = mcp_access::server_scope(&server).unwrap();
    let tool = mcp::run(server.clone(), Action::Inspect, token())
        .unwrap()
        .tools
        .into_iter()
        .find(|item| item["name"] == "search_knowledge")
        .unwrap();
    Store::load(access_path.clone())
        .set(&scope, &tool, Some(Rule::AllowDeclaredReadOnly))
        .unwrap();
    let (endpoint, fixture) = model_server(vec![
        plan_reply(),
        json!({"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"search_knowledge","arguments":{"query":"synthetic"}}}]}}),
        answer_reply(),
    ]);
    let mut config = config(endpoint, access_path, 1);
    config.server = server;
    config.selected = vec!["search_knowledge".into()];
    let plan = agent::prepare(config, token()).unwrap();
    let outcome = agent::execute(plan, token(), |_| {}).unwrap();
    assert_eq!(outcome.steps[0].references.len(), 1);
    let reference = &outcome.steps[0].references[0];
    assert_eq!(reference.relative_path, "guide.md");
    assert_eq!(reference.file_sha256, "a".repeat(64));
    assert!(!format!("{reference:?}").contains("synthetic excerpt"));
    assert_eq!(fixture.join().unwrap().len(), 3);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn refuses_out_of_scope_tool_and_revoked_grant() {
    let (root, access_path, tool) = approved();
    let (endpoint, fixture) = model_server(vec![plan_reply(), call_reply("delete_file")]);
    let plan = agent::prepare(config(endpoint, access_path.clone(), 2), token()).unwrap();
    let error = agent::execute(plan, token(), |_| {})
        .unwrap_err()
        .to_string();
    assert!(error.contains("白名单"), "{error}");
    assert_eq!(fixture.join().unwrap().len(), 2);

    let (endpoint, fixture) = model_server(vec![plan_reply()]);
    let plan = agent::prepare(config(endpoint, access_path.clone(), 2), token()).unwrap();
    let scope = mcp_access::server_scope(&server()).unwrap();
    Store::load(access_path.clone())
        .set(&scope, &tool, None)
        .unwrap();
    let error = agent::execute(plan, token(), |_| {})
        .unwrap_err()
        .to_string();
    assert!(error.contains("未获直接许可"), "{error}");
    assert_eq!(fixture.join().unwrap().len(), 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn enforces_call_budget_and_cancellation() {
    let (root, access_path, _) = approved();
    let (endpoint, fixture) =
        model_server(vec![plan_reply(), call_reply("echo"), call_reply("echo")]);
    let plan = agent::prepare(config(endpoint, access_path.clone(), 1), token()).unwrap();
    let error = agent::execute(plan, token(), |_| {})
        .unwrap_err()
        .to_string();
    assert!(error.contains("上限"), "{error}");
    assert_eq!(fixture.join().unwrap().len(), 3);

    let cancelled = token();
    cancelled.store(true, Ordering::Relaxed);
    let error = agent::prepare(
        config("http://127.0.0.1:11434/api/chat".into(), access_path, 1),
        cancelled,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("取消"), "{error}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cancellation_interrupts_an_in_flight_model_request() {
    let (root, access_path, tool) = approved();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/api/chat", listener.local_addr().unwrap());
    let (accepted_tx, accepted_rx) = mpsc::channel();
    let server_thread = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut bytes = [0u8; 4096];
        assert!(stream.read(&mut bytes).unwrap() > 0);
        accepted_tx.send(()).unwrap();
        std::thread::sleep(Duration::from_secs(2));
    });
    let plan = agent::Plan {
        text: "Call echo once".into(),
        config: config(endpoint, access_path, 1),
        tools: vec![tool],
    };
    let cancelled = token();
    let flag = Arc::clone(&cancelled);
    let worker = std::thread::spawn(move || {
        agent::execute(plan, cancelled, |_| {})
            .unwrap_err()
            .to_string()
    });
    accepted_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let started = Instant::now();
    flag.store(true, Ordering::Relaxed);
    let error = worker.join().unwrap();
    assert!(error.contains("取消"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(1));
    server_thread.join().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "requires local Ollama qwen2.5:7b; run explicitly for stage acceptance"]
fn local_ollama_uses_disposable_read_only_tool() {
    let (root, access_path, _) = approved();
    let mut config = config("http://127.0.0.1:11434/api/chat".into(), access_path, 2);
    config.model = "qwen2.5:7b".into();
    config.goal = "Call the echo tool exactly once with text 'synthetic Rust note', then report the returned text in Chinese. Do not answer from memory.".into();
    let plan = agent::prepare(config, token()).unwrap();
    let result = agent::execute(plan, token(), |_| {}).unwrap();
    assert_eq!(result.steps.len(), 1);
    assert!(result.steps[0].result.contains("内容项"));
    assert!(!result.answer.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}
