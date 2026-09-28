use serde_json::json;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use zi_devtools::mcp::{self, Action, Config};

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
        .expect("Python must be available for the disposable MCP protocol fixture")
}

fn config(mode: &str) -> Config {
    Config {
        executable: python(),
        args: vec![
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/mcp_stdio_server.py")
                .to_string_lossy()
                .into_owned(),
            mode.into(),
        ],
    }
}

fn token() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

#[test]
fn initializes_paginates_and_calls_only_listed_tools() {
    let report = mcp::run(config("normal"), Action::Inspect, token()).unwrap();
    assert_eq!(report.server, "Zi test MCP");
    assert_eq!(report.protocol, "2025-06-18");
    assert_eq!(report.tools.len(), 2);
    assert_eq!(report.resources.len(), 1);
    assert_eq!(report.prompts.len(), 1);

    let called = mcp::run(
        config("normal"),
        Action::Call {
            tool: "echo".into(),
            arguments: json!({"text":"中文 MCP"}),
        },
        token(),
    )
    .unwrap();
    assert_eq!(
        called
            .call_result
            .as_ref()
            .unwrap()
            .pointer("/content/0/text"),
        Some(&json!("中文 MCP"))
    );
    let error = mcp::run(
        config("normal"),
        Action::Call {
            tool: "unlisted".into(),
            arguments: json!({}),
        },
        token(),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("没有列出"));
}

#[test]
fn cancellation_and_oversize_end_the_disposable_server() {
    let cancellation = token();
    let trigger = Arc::clone(&cancellation);
    let handle = std::thread::spawn(move || {
        mcp::run(config("hang"), Action::Inspect, cancellation)
            .unwrap_err()
            .to_string()
    });
    std::thread::sleep(Duration::from_millis(250));
    let start = Instant::now();
    trigger.store(true, Ordering::Relaxed);
    let message = handle.join().unwrap();
    assert!(message.contains("已停止"), "{message}");
    assert!(start.elapsed() < Duration::from_secs(2));

    let message = mcp::run(config("oversize"), Action::Inspect, token())
        .unwrap_err()
        .to_string();
    assert!(message.contains("1 MiB"), "{message}");
}

#[test]
fn rejects_unsupported_version_and_invalid_capability_entries() {
    let message = mcp::run(config("unsupported"), Action::Inspect, token())
        .unwrap_err()
        .to_string();
    assert!(message.contains("不受支持"), "{message}");

    let report = mcp::run(config("no_tools"), Action::Inspect, token()).unwrap();
    assert!(report.tools.is_empty());
    assert_eq!(report.resources.len(), 1);

    let message = mcp::run(config("bad_tool"), Action::Inspect, token())
        .unwrap_err()
        .to_string();
    assert!(message.contains("无效名称"), "{message}");
}
