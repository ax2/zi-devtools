use serde_json::json;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use zi_devtools::mcp::{self, Action, Config};
use zi_devtools::mcp_access::{self, Decision, Rule, Store};

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
fn connected_stdio_reuses_process_and_rechecks_permissions() {
    let root = std::env::temp_dir().join(format!("zi-mcp-connected-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let access_path = root.join("mcp-permissions.json");
    let config = config("identity");
    let scope = mcp_access::server_scope(&config).unwrap();
    let cancelled = token();
    let (sender, requests) = mpsc::channel();
    let worker = std::thread::spawn({
        let cancelled = Arc::clone(&cancelled);
        let access_path = access_path.clone();
        let config = config.clone();
        move || mcp::serve_connected(config, cancelled, access_path, requests).unwrap()
    });
    let run = |action, manual_confirmed| {
        let (response, received) = mpsc::channel();
        sender
            .send(mcp::ConnectedRequest {
                action,
                manual_confirmed,
                response,
            })
            .unwrap();
        received.recv_timeout(Duration::from_secs(10)).unwrap()
    };
    let first = run(Action::Inspect, false).unwrap();
    let resource = || Action::ReadResource {
        uri: "fixture://guide".into(),
    };
    let pid = |report: mcp::Report| {
        report.resource_result.unwrap().1["contents"][0]["text"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(
        pid(run(resource(), false).unwrap()),
        pid(run(resource(), false).unwrap())
    );
    let tool = first.tools[1].clone();
    let call = || Action::Call {
        tool: "echo".into(),
        arguments: json!({"text":"same process"}),
        expected_tool: tool.clone(),
    };
    assert!(run(call(), true).unwrap().call_result.is_some());
    Store::load(access_path.clone())
        .set(&scope, &tool, Some(Rule::Deny))
        .unwrap();
    assert!(run(call(), true).unwrap_err().contains("禁止"));
    worker.join().unwrap();
    assert!(
        sender
            .send(mcp::ConnectedRequest {
                action: Action::Inspect,
                manual_confirmed: false,
                response: mpsc::channel().0,
            })
            .is_err()
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn connected_stdio_cancel_stops_an_in_flight_request() {
    let config = config("hang_resource");
    let cancelled = token();
    let (sender, requests) = mpsc::channel();
    let worker = std::thread::spawn({
        let cancelled = Arc::clone(&cancelled);
        move || {
            mcp::serve_connected(
                config,
                cancelled,
                std::env::temp_dir().join("unused-mcp-permissions.json"),
                requests,
            )
            .unwrap()
        }
    });
    let ask = |action| {
        let (response, received) = mpsc::channel();
        sender
            .send(mcp::ConnectedRequest {
                action,
                manual_confirmed: false,
                response,
            })
            .unwrap();
        received
    };
    ask(Action::Inspect)
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap();
    let started = Instant::now();
    let response = ask(Action::ReadResource {
        uri: "fixture://guide".into(),
    });
    std::thread::sleep(Duration::from_millis(100));
    cancelled.store(true, Ordering::Relaxed);
    assert!(
        response
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .is_err()
    );
    worker.join().unwrap();
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn durable_tool_permissions_bind_server_and_definition_and_fail_closed() {
    let root = std::env::temp_dir().join(format!("zi-mcp-access-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("mcp-permissions.json");
    let server = config("normal");
    let scope = mcp_access::server_scope(&server).unwrap();
    let inspected = mcp::run(server.clone(), Action::Inspect, token()).unwrap();
    let tool = inspected.tools[1].clone();
    assert!(mcp_access::declared_low_impact(&tool));
    let call = || Action::Call {
        tool: "echo".into(),
        arguments: json!({"text":"permission fixture"}),
        expected_tool: tool.clone(),
    };
    let mut store = Store::load(path.clone());
    assert_eq!(store.decision(&scope, &tool).unwrap(), Decision::Confirm);
    assert!(
        mcp::run_with_access(server.clone(), call(), token(), path.clone(), false)
            .unwrap_err()
            .to_string()
            .contains("人工确认")
    );
    assert!(
        mcp::run_with_access(server.clone(), call(), token(), path.clone(), true)
            .unwrap()
            .call_result
            .is_some()
    );
    store
        .set(&scope, &tool, Some(Rule::AllowDeclaredReadOnly))
        .unwrap();
    assert_eq!(
        Store::load(path.clone()).decision(&scope, &tool).unwrap(),
        Decision::Direct
    );
    assert!(
        mcp::run_with_access(server.clone(), call(), token(), path.clone(), false)
            .unwrap()
            .call_result
            .is_some()
    );
    let mut changed = tool.clone();
    changed["description"] = json!("changed by server");
    assert_eq!(store.decision(&scope, &changed).unwrap(), Decision::Confirm);
    let mut unsafe_tool = tool.clone();
    unsafe_tool["annotations"]["openWorldHint"] = json!(true);
    assert!(
        store
            .set(&scope, &unsafe_tool, Some(Rule::AllowDeclaredReadOnly))
            .is_err()
    );
    store.set(&scope, &tool, Some(Rule::Deny)).unwrap();
    assert_eq!(store.decision(&scope, &changed).unwrap(), Decision::Deny);
    assert!(
        mcp::run_with_access(server.clone(), call(), token(), path.clone(), true)
            .unwrap_err()
            .to_string()
            .contains("禁止调用")
    );
    store.set(&scope, &tool, None).unwrap();
    assert_eq!(store.decision(&scope, &tool).unwrap(), Decision::Confirm);

    std::fs::write(&path, b"{broken").unwrap();
    let original = std::fs::read(&path).unwrap();
    let mut broken = Store::load(path.clone());
    assert!(broken.error.is_some());
    assert!(broken.set(&scope, &tool, Some(Rule::Deny)).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(
        mcp::run_with_access(server, call(), token(), path.clone(), true)
            .unwrap_err()
            .to_string()
            .contains("权限文件")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn revocation_during_listing_prevents_the_tool_call() {
    let root = std::env::temp_dir().join(format!("zi-mcp-revoke-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("mcp-permissions.json");
    let server = config("delayed_tools");
    let scope = mcp_access::server_scope(&server).unwrap();
    let inspected = mcp::run(server.clone(), Action::Inspect, token()).unwrap();
    let tool = inspected.tools[1].clone();
    let mut store = Store::load(path.clone());
    store
        .set(&scope, &tool, Some(Rule::AllowDeclaredReadOnly))
        .unwrap();
    let call_path = path.clone();
    let call_tool = tool.clone();
    let started = Instant::now();
    let handle = std::thread::spawn(move || {
        mcp::run_with_access(
            server,
            Action::Call {
                tool: "echo".into(),
                arguments: json!({"text":"must not run"}),
                expected_tool: call_tool,
            },
            token(),
            call_path,
            false,
        )
        .unwrap_err()
        .to_string()
    });
    std::thread::sleep(Duration::from_millis(220));
    store.set(&scope, &tool, None).unwrap();
    let message = handle.join().unwrap();
    assert!(message.contains("重新人工确认"), "{message}");
    assert!(started.elapsed() >= Duration::from_millis(650));
    std::fs::remove_dir_all(root).unwrap();
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
            expected_tool: report.tools[1].clone(),
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
            expected_tool: json!({"name":"unlisted"}),
        },
        token(),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("没有列出"));
}

#[test]
fn refuses_changed_tool_definition_before_call() {
    let inspected = mcp::run(config("normal"), Action::Inspect, token()).unwrap();
    for mode in ["changed_tool", "changed_schema", "changed_annotations"] {
        let error = mcp::run(
            config(mode),
            Action::Call {
                tool: "echo".into(),
                arguments: json!({"text":"fixture"}),
                expected_tool: inspected.tools[1].clone(),
            },
            token(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("定义已变化"), "{mode}: {error}");
    }
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

#[test]
fn reads_only_listed_resources_and_prompts_with_bounded_arguments() {
    let resource = mcp::run(
        config("normal"),
        Action::ReadResource {
            uri: "fixture://guide".into(),
        },
        token(),
    )
    .unwrap();
    assert_eq!(
        resource
            .resource_result
            .as_ref()
            .unwrap()
            .1
            .pointer("/contents/0/text"),
        Some(&json!("本地测试指南"))
    );

    let prompt = mcp::run(
        config("normal"),
        Action::GetPrompt {
            name: "summary".into(),
            arguments: json!({"topic":"Rust"}),
        },
        token(),
    )
    .unwrap();
    assert_eq!(
        prompt
            .prompt_result
            .as_ref()
            .unwrap()
            .1
            .pointer("/messages/0/content/text"),
        Some(&json!("概括：Rust"))
    );

    for (action, expected) in [
        (
            Action::ReadResource {
                uri: "fixture://unlisted".into(),
            },
            "没有列出该资源",
        ),
        (
            Action::GetPrompt {
                name: "unlisted".into(),
                arguments: json!({}),
            },
            "没有列出该提示词",
        ),
        (
            Action::GetPrompt {
                name: "summary".into(),
                arguments: json!({"topic":42}),
            },
            "字符串值",
        ),
    ] {
        let error = mcp::run(config("normal"), action, token())
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "{error}");
    }
    assert!(
        mcp::run(
            config("bad_resource_result"),
            Action::ReadResource {
                uri: "fixture://guide".into(),
            },
            token()
        )
        .unwrap_err()
        .to_string()
        .contains("contents")
    );
    assert!(
        mcp::run(
            config("bad_prompt_result"),
            Action::GetPrompt {
                name: "summary".into(),
                arguments: json!({}),
            },
            token()
        )
        .unwrap_err()
        .to_string()
        .contains("messages")
    );
}

#[test]
fn cancelling_resource_read_exits_the_disposable_server() {
    let cancellation = token();
    let trigger = Arc::clone(&cancellation);
    let handle = std::thread::spawn(move || {
        mcp::run(
            config("hang_resource"),
            Action::ReadResource {
                uri: "fixture://guide".into(),
            },
            cancellation,
        )
        .unwrap_err()
        .to_string()
    });
    std::thread::sleep(Duration::from_millis(300));
    let start = Instant::now();
    trigger.store(true, Ordering::Relaxed);
    let message = handle.join().unwrap();
    assert!(message.contains("已停止"), "{message}");
    assert!(start.elapsed() < Duration::from_secs(2));
}
