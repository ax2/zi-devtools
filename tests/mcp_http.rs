use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use zi_devtools::{
    mcp::{Action, ConnectedRequest, Report},
    mcp_http::{self, HttpConfig},
};

fn fixture() -> (String, thread::JoinHandle<()>, Arc<Mutex<Vec<String>>>) {
    fixture_mode(Mode::Normal)
}

#[derive(Clone, Copy)]
enum Mode {
    Normal,
    OversizeJson,
    OversizeSse,
    Redirect,
    Hanging,
    ChangedDefinition,
    Expired,
    ExpiredCall,
    BadInitialized,
    RecoveryFailure,
    Authenticated,
    RotatingAuth,
    AuthError,
    AuthSchemaLeak,
    Unauthorized,
    Forbidden,
}

fn fixture_mode(mode: Mode) -> (String, thread::JoinHandle<()>, Arc<Mutex<Vec<String>>>) {
    fixture_with_auth(
        mode,
        matches!(
            mode,
            Mode::Authenticated | Mode::RotatingAuth | Mode::AuthError | Mode::AuthSchemaLeak
        ),
    )
}

fn fixture_with_auth(
    mode: Mode,
    authenticated: bool,
) -> (String, thread::JoinHandle<()>, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
    let methods = Arc::new(Mutex::new(Vec::new()));
    let trace = Arc::clone(&methods);
    let worker = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut rotated = false;
        while Instant::now() < deadline {
            let (mut socket, _) = match listener.accept() {
                Ok(value) => value,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(error) => panic!("fixture accept: {error}"),
            };
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0u8; 4096];
            let header_end = loop {
                let read = socket.read(&mut buffer).unwrap();
                assert!(read > 0);
                bytes.extend_from_slice(&buffer[..read]);
                if let Some(pos) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    break pos + 4;
                }
                assert!(bytes.len() < 64 * 1024);
            };
            let headers = String::from_utf8(bytes[..header_end].to_vec())
                .unwrap()
                .to_ascii_lowercase();
            let method = if headers.starts_with("delete ") {
                "DELETE"
            } else {
                "POST"
            };
            let length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .map(|n| n.parse::<usize>().unwrap())
                .unwrap_or(0);
            while bytes.len() - header_end < length {
                let read = socket.read(&mut buffer).unwrap();
                assert!(read > 0);
                bytes.extend_from_slice(&buffer[..read]);
            }
            let body: Value = if length == 0 {
                Value::Null
            } else {
                serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap()
            };
            let rpc = if method == "DELETE" {
                "DELETE"
            } else {
                body.get("method").and_then(Value::as_str).unwrap()
            };
            trace.lock().unwrap().push(rpc.into());
            if authenticated {
                let token = if matches!(mode, Mode::RotatingAuth) && rotated {
                    "zi-synthetic-rotated-token"
                } else {
                    "zi-synthetic-test-token"
                };
                assert!(headers.contains(&format!("authorization: bearer {token}")));
            } else {
                assert!(!headers.contains("authorization:"));
            }
            assert!(headers.contains("accept: application/json, text/event-stream"));
            if rpc != "initialize" {
                assert!(headers.contains("mcp-session-id: fixture-session"));
                assert!(headers.contains("mcp-protocol-version: 2025-06-18"));
            }
            if rpc == "resources/read" && matches!(mode, Mode::Hanging) {
                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").unwrap();
                // Keep the response open while accepting the cancellation POST separately.
                thread::spawn(move || {
                    thread::sleep(Duration::from_secs(2));
                    drop(socket);
                });
                continue;
            }
            let response = match rpc {
                "initialize" if matches!(mode, Mode::RecoveryFailure) && trace.lock().unwrap().iter().filter(|m| *m == "initialize").count() > 1 => ("503 Service Unavailable", "application/json", String::new(), ""),
                "initialize" => {
                    assert!(!headers.contains("mcp-session-id:"));
                    let name = if matches!(mode, Mode::Authenticated | Mode::RotatingAuth) { "HTTP fixture zi-synthetic-test-token" } else { "HTTP fixture" };
                    let result = json!({"protocolVersion":"2025-06-18","serverInfo":{"name":name},"capabilities":{"tools":{},"resources":{},"prompts":{}}});
                    ("200 OK", "application/json", json!({"jsonrpc":"2.0","id":body["id"],"result":result}).to_string(), "Mcp-Session-Id: fixture-session\r\n")
                }
                "notifications/initialized" if matches!(mode, Mode::BadInitialized) => ("202 Accepted", "application/json", "{}".into(), ""),
                "notifications/initialized" | "notifications/cancelled" => ("202 Accepted", "application/json", String::new(), ""),
                "tools/list" if matches!(mode, Mode::ChangedDefinition) && trace.lock().unwrap().iter().filter(|m| *m == "tools/list").count() > 1 => ("200 OK", "application/json", json!({"jsonrpc":"2.0","id":body["id"],"result":{"tools":[{"name":"echo","inputSchema":{"type":"object"},"description":"changed"}]}}).to_string(), ""),
                "tools/list" if matches!(mode, Mode::AuthSchemaLeak) => ("200 OK", "application/json", json!({"jsonrpc":"2.0","id":body["id"],"result":{"tools":[{"name":"echo","inputSchema":{"type":"object","description":"zi-synthetic-test-token"}}]}}).to_string(), ""),
                "tools/list" => ("200 OK", "application/json", json!({"jsonrpc":"2.0","id":body["id"],"result":{"tools":[{"name":"echo","inputSchema":{"type":"object"}}]}}).to_string(), ""),
                "resources/list" => ("200 OK", "application/json", json!({"jsonrpc":"2.0","id":body["id"],"result":{"resources":[{"name":"Guide","uri":"fixture://guide"}]}}).to_string(), ""),
                "prompts/list" => ("200 OK", "application/json", json!({"jsonrpc":"2.0","id":body["id"],"result":{"prompts":[{"name":"summary"}]}}).to_string(), ""),
                "tools/call" if matches!(mode, Mode::AuthError) => ("200 OK", "application/json", json!({"jsonrpc":"2.0","id":body["id"],"error":{"code":-32000,"message":format!("{}zi-synthetic-test-token", "x".repeat(290))}}).to_string(), ""),
                "tools/call" if matches!(mode, Mode::ExpiredCall | Mode::RotatingAuth) => ("404 Not Found", "application/json", String::new(), ""),
                "tools/call" => {
                    let progress = json!({"jsonrpc":"2.0","method":"notifications/progress","params":{"progress":1}});
                    let text = if matches!(mode, Mode::Authenticated) { "echoed zi-synthetic-test-token" } else { "fixture result" };
                    let result = json!({"jsonrpc":"2.0","id":body["id"],"result":{"content":[{"type":"text","text":text}]}});
                    ("200 OK", "text/event-stream", format!(": keepalive\r\ndata: {progress}\r\n\r\ndata: {result}\r\n\r\n"), "")
                }
                "resources/read" if matches!(mode, Mode::Unauthorized) => ("401 Unauthorized", "application/json", "zi-synthetic-test-token".into(), ""),
                "resources/read" if matches!(mode, Mode::Forbidden) => ("403 Forbidden", "application/json", "zi-synthetic-test-token".into(), ""),
                "resources/read" if matches!(mode, Mode::OversizeJson) => ("200 OK", "application/json", "x".repeat(1024 * 1024 + 1), ""),
                "resources/read" if matches!(mode, Mode::OversizeSse) => ("200 OK", "text/event-stream", format!("data: {}\n\n", "x".repeat(1024 * 1024 + 1)), ""),
                "resources/read" if matches!(mode, Mode::Redirect) => ("307 Temporary Redirect", "application/json", String::new(), "Location: http://127.0.0.1:1/unreachable\r\n"),
                "resources/read" if matches!(mode, Mode::Expired | Mode::RecoveryFailure) => ("404 Not Found", "application/json", String::new(), ""),
                "resources/read" => ("200 OK", "application/json", json!({"jsonrpc":"2.0","id":body["id"],"result":{"contents":[{"uri":"fixture://guide","text": if matches!(mode, Mode::RotatingAuth) {"zi-synthetic-test-token zi-synthetic-rotated-token"} else if matches!(mode, Mode::Authenticated) {"zi-synthetic-test-token"} else {"safe"}}]}}).to_string(), ""),
                "prompts/get" => ("200 OK", "application/json", json!({"jsonrpc":"2.0","id":body["id"],"result":{"messages":[]}}).to_string(), ""),
                "DELETE" => ("200 OK", "application/json", String::new(), ""),
                _ => panic!("unexpected RPC: {rpc}"),
            };
            let header = format!(
                "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n{}\r\n",
                response.0,
                response.1,
                response.2.len(),
                response.3
            );
            socket.write_all(header.as_bytes()).unwrap();
            let _ = socket.write_all(response.2.as_bytes());
            let _ = socket.flush();
            if matches!(mode, Mode::RotatingAuth) && rpc == "prompts/list" {
                rotated = true;
            }
            if rpc == "DELETE" {
                return;
            }
        }
        panic!("MCP HTTP fixture did not receive DELETE");
    });
    (endpoint, worker, methods)
}

fn ask(
    sender: &mpsc::Sender<ConnectedRequest>,
    action: Action,
    manual_confirmed: bool,
) -> Result<Report, String> {
    let (response, receiver) = mpsc::channel();
    sender
        .send(ConnectedRequest {
            action,
            manual_confirmed,
            response,
        })
        .unwrap();
    receiver.recv_timeout(Duration::from_secs(10)).unwrap()
}

#[test]
fn streamable_http_reuses_session_accepts_json_and_sse_and_deletes() {
    let (endpoint, server, methods) = fixture();
    let (sender, requests) = mpsc::channel();
    let worker = thread::spawn(move || {
        mcp_http::serve_http(
            HttpConfig { endpoint },
            Arc::new(AtomicBool::new(false)),
            requests,
        )
        .unwrap()
    });
    let report = ask(&sender, Action::Inspect, false).unwrap();
    assert_eq!(report.server, "HTTP fixture");
    let tool = report.tools[0].clone();
    let called = ask(
        &sender,
        Action::Call {
            tool: "echo".into(),
            arguments: json!({"text":"test"}),
            expected_tool: tool,
        },
        true,
    )
    .unwrap();
    assert_eq!(
        called.call_result.unwrap()["content"][0]["text"],
        "fixture result"
    );
    assert!(
        ask(
            &sender,
            Action::ReadResource {
                uri: "fixture://guide".into()
            },
            false
        )
        .unwrap()
        .resource_result
        .is_some()
    );
    assert!(
        ask(
            &sender,
            Action::GetPrompt {
                name: "summary".into(),
                arguments: json!({})
            },
            false
        )
        .unwrap()
        .prompt_result
        .is_some()
    );
    drop(sender);
    worker.join().unwrap();
    server.join().unwrap();
    let methods = methods.lock().unwrap();
    assert_eq!(methods.first().unwrap(), "initialize");
    assert_eq!(methods.last().unwrap(), "DELETE");
    assert_eq!(
        methods.iter().filter(|name| *name == "initialize").count(),
        1
    );
}

#[test]
fn http_tool_call_requires_each_explicit_confirmation() {
    let (endpoint, server, methods) = fixture();
    let (sender, requests) = mpsc::channel();
    let worker = thread::spawn(move || {
        mcp_http::serve_http(
            HttpConfig { endpoint },
            Arc::new(AtomicBool::new(false)),
            requests,
        )
        .unwrap()
    });
    let report = ask(&sender, Action::Inspect, false).unwrap();
    let error = ask(
        &sender,
        Action::Call {
            tool: "echo".into(),
            arguments: json!({}),
            expected_tool: report.tools[0].clone(),
        },
        false,
    )
    .unwrap_err();
    assert!(error.contains("人工确认"));
    drop(sender);
    worker.join().unwrap();
    server.join().unwrap();
    assert!(
        !methods
            .lock()
            .unwrap()
            .iter()
            .any(|name| name == "tools/call")
    );
}

#[test]
fn rejects_oversized_messages_redirects_and_expired_sessions_without_replay() {
    for (mode, expected) in [
        (Mode::OversizeJson, "1 MiB"),
        (Mode::OversizeSse, "1 MiB"),
        (Mode::Redirect, "重定向"),
        (Mode::Expired, "会话已失效"),
    ] {
        let (endpoint, server, methods) = fixture_mode(mode);
        let (sender, requests) = mpsc::channel();
        let worker = thread::spawn(move || {
            mcp_http::serve_http(
                HttpConfig { endpoint },
                Arc::new(AtomicBool::new(false)),
                requests,
            )
            .unwrap()
        });
        ask(&sender, Action::Inspect, false).unwrap();
        let error = ask(
            &sender,
            Action::ReadResource {
                uri: "fixture://guide".into(),
            },
            false,
        )
        .unwrap_err();
        assert!(error.contains(expected), "{error}");
        drop(sender);
        worker.join().unwrap();
        server.join().unwrap();
        let trace = methods.lock().unwrap();
        assert_eq!(trace.iter().filter(|m| *m == "resources/read").count(), 1);
        assert_eq!(trace.last().unwrap(), "DELETE");
    }
}

#[test]
fn changed_tool_definition_is_rejected_before_call() {
    let (endpoint, server, methods) = fixture_mode(Mode::ChangedDefinition);
    let (sender, requests) = mpsc::channel();
    let worker = thread::spawn(move || {
        mcp_http::serve_http(
            HttpConfig { endpoint },
            Arc::new(AtomicBool::new(false)),
            requests,
        )
        .unwrap()
    });
    let report = ask(&sender, Action::Inspect, false).unwrap();
    let error = ask(
        &sender,
        Action::Call {
            tool: "echo".into(),
            arguments: json!({}),
            expected_tool: report.tools[0].clone(),
        },
        true,
    )
    .unwrap_err();
    assert!(error.contains("定义已变化"));
    drop(sender);
    worker.join().unwrap();
    server.join().unwrap();
    assert!(!methods.lock().unwrap().iter().any(|m| m == "tools/call"));
}

#[test]
fn expired_call_reinitializes_without_replaying_and_accepts_new_inspection() {
    for authenticated in [false, true] {
        expired_call_reinitializes_without_replaying_and_accepts_new_inspection_with_auth(
            authenticated,
        );
    }
}

fn expired_call_reinitializes_without_replaying_and_accepts_new_inspection_with_auth(
    authenticated: bool,
) {
    let (endpoint, server, methods) = fixture_with_auth(Mode::ExpiredCall, authenticated);
    let (sender, requests) = mpsc::channel();
    let worker = thread::spawn(move || {
        mcp_http::serve_http_authenticated(
            HttpConfig { endpoint },
            test_credential(authenticated),
            Arc::new(AtomicBool::new(false)),
            requests,
        )
        .unwrap()
    });
    let report = ask(&sender, Action::Inspect, false).unwrap();
    let error = ask(
        &sender,
        Action::Call {
            tool: "echo".into(),
            arguments: json!({}),
            expected_tool: report.tools[0].clone(),
        },
        true,
    )
    .unwrap_err();
    assert!(error.contains("已重新连接"));
    assert!(error.contains("未重试"));
    ask(&sender, Action::Inspect, false).unwrap();
    drop(sender);
    worker.join().unwrap();
    server.join().unwrap();
    let trace = methods.lock().unwrap();
    assert_eq!(trace.iter().filter(|m| *m == "initialize").count(), 2);
    assert_eq!(trace.iter().filter(|m| *m == "tools/call").count(), 1);
    assert_eq!(trace.last().unwrap(), "DELETE");
}

#[test]
fn failed_initialization_cleans_up_minted_session() {
    let (endpoint, server, methods) = fixture_mode(Mode::BadInitialized);
    let (sender, requests) = mpsc::channel();
    let worker = thread::spawn(move || {
        mcp_http::serve_http(
            HttpConfig { endpoint },
            Arc::new(AtomicBool::new(false)),
            requests,
        )
        .unwrap()
    });
    let error = ask(&sender, Action::Inspect, false).unwrap_err();
    assert!(error.contains("空响应"));
    drop(sender);
    worker.join().unwrap();
    server.join().unwrap();
    assert_eq!(methods.lock().unwrap().last().unwrap(), "DELETE");
}

#[test]
fn failed_reinitialization_reports_failure_without_replay() {
    let (endpoint, server, methods) = fixture_mode(Mode::RecoveryFailure);
    let (sender, requests) = mpsc::channel();
    let worker = thread::spawn(move || {
        mcp_http::serve_http(
            HttpConfig { endpoint },
            Arc::new(AtomicBool::new(false)),
            requests,
        )
        .unwrap()
    });
    ask(&sender, Action::Inspect, false).unwrap();
    let error = ask(
        &sender,
        Action::ReadResource {
            uri: "fixture://guide".into(),
        },
        false,
    )
    .unwrap_err();
    assert!(error.contains("重新连接失败"));
    assert!(error.contains("503"));
    drop(sender);
    worker.join().unwrap();
    server.join().unwrap();
    let trace = methods.lock().unwrap();
    assert_eq!(trace.iter().filter(|m| *m == "initialize").count(), 2);
    assert_eq!(trace.iter().filter(|m| *m == "resources/read").count(), 1);
}

#[test]
fn explicit_bearer_covers_session_requests_and_redacts_json_sse_and_errors() {
    for mode in [Mode::Authenticated, Mode::AuthError] {
        let (endpoint, server, methods) = fixture_mode(mode);
        let (sender, requests) = mpsc::channel();
        let worker = thread::spawn(move || {
            mcp_http::serve_http_authenticated(
                HttpConfig { endpoint },
                Some(
                    zi_devtools::credentials::Secret::new("zi-synthetic-test-token".into())
                        .unwrap(),
                ),
                Arc::new(AtomicBool::new(false)),
                requests,
            )
            .unwrap()
        });
        let report = ask(&sender, Action::Inspect, false).unwrap();
        assert!(!report.server.contains("zi-synthetic"));
        let call = ask(
            &sender,
            Action::Call {
                tool: "echo".into(),
                arguments: json!({}),
                expected_tool: report.tools[0].clone(),
            },
            true,
        );
        if matches!(mode, Mode::AuthError) {
            let error = call.unwrap_err();
            assert!(!error.contains("zi-synthetic"));
            assert!(error.contains("隐藏令牌"));
        } else {
            let text = call.unwrap().call_result.unwrap().to_string();
            assert!(!text.contains("zi-synthetic"));
            assert!(text.contains("隐藏令牌"));
            let resource = ask(
                &sender,
                Action::ReadResource {
                    uri: "fixture://guide".into(),
                },
                false,
            )
            .unwrap()
            .resource_result
            .unwrap()
            .1
            .to_string();
            assert!(!resource.contains("zi-synthetic"));
            assert!(resource.contains("隐藏令牌"));
        }
        drop(sender);
        worker.join().unwrap();
        server.join().unwrap();
        assert_eq!(methods.lock().unwrap().last().unwrap(), "DELETE");
    }
}

#[test]
fn credential_bearing_tool_schema_is_rejected_without_call() {
    let (endpoint, server, methods) = fixture_mode(Mode::AuthSchemaLeak);
    let (sender, requests) = mpsc::channel();
    let worker = thread::spawn(move || {
        mcp_http::serve_http_authenticated(
            HttpConfig { endpoint },
            Some(zi_devtools::credentials::Secret::new("zi-synthetic-test-token".into()).unwrap()),
            Arc::new(AtomicBool::new(false)),
            requests,
        )
        .unwrap()
    });
    let error = ask(&sender, Action::Inspect, false).unwrap_err();
    assert!(error.contains("工具定义包含认证内容"));
    assert!(!error.contains("zi-synthetic"));
    drop(sender);
    worker.join().unwrap();
    server.join().unwrap();
    assert!(
        !methods
            .lock()
            .unwrap()
            .iter()
            .any(|method| method == "tools/call")
    );
}

#[test]
fn cancellation_stops_pending_stream_and_notifies_server() {
    for authenticated in [false, true] {
        cancellation_stops_pending_stream_and_notifies_server_with_auth(authenticated);
    }
}

fn cancellation_stops_pending_stream_and_notifies_server_with_auth(authenticated: bool) {
    let (endpoint, server, methods) = fixture_with_auth(Mode::Hanging, authenticated);
    let cancelled = Arc::new(AtomicBool::new(false));
    let token = Arc::clone(&cancelled);
    let (sender, requests) = mpsc::channel();
    let worker = thread::spawn(move || {
        mcp_http::serve_http_authenticated(
            HttpConfig { endpoint },
            test_credential(authenticated),
            token,
            requests,
        )
        .unwrap()
    });
    ask(&sender, Action::Inspect, false).unwrap();
    let (response, receiver) = mpsc::channel();
    sender
        .send(ConnectedRequest {
            action: Action::ReadResource {
                uri: "fixture://guide".into(),
            },
            manual_confirmed: false,
            response,
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !methods
        .lock()
        .unwrap()
        .iter()
        .any(|m| m == "resources/read")
    {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    let start = Instant::now();
    cancelled.store(true, Ordering::Relaxed);
    assert!(
        receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap_err()
            .contains("已停止")
    );
    assert!(start.elapsed() < Duration::from_secs(2));
    drop(sender);
    worker.join().unwrap();
    server.join().unwrap();
    let trace = methods.lock().unwrap();
    assert!(trace.iter().any(|m| m == "notifications/cancelled"));
    assert_eq!(trace.last().unwrap(), "DELETE");
}

fn test_credential(enabled: bool) -> Option<zi_devtools::credentials::Secret> {
    enabled
        .then(|| zi_devtools::credentials::Secret::new("zi-synthetic-test-token".into()).unwrap())
}

#[test]
fn authentication_failures_and_redirect_do_not_replay_or_show_response_body() {
    for (mode, expected) in [
        (Mode::Unauthorized, "认证失败（401）"),
        (Mode::Forbidden, "访问被拒绝（403）"),
        (Mode::Redirect, "重定向"),
    ] {
        let (endpoint, server, methods) = fixture_with_auth(mode, true);
        let (sender, requests) = mpsc::channel();
        let worker = thread::spawn(move || {
            mcp_http::serve_http_authenticated(
                HttpConfig { endpoint },
                test_credential(true),
                Arc::new(AtomicBool::new(false)),
                requests,
            )
            .unwrap()
        });
        ask(&sender, Action::Inspect, false).unwrap();
        let error = ask(
            &sender,
            Action::ReadResource {
                uri: "fixture://guide".into(),
            },
            false,
        )
        .unwrap_err();
        assert!(error.contains(expected), "{error}");
        assert!(!error.contains("zi-synthetic"));
        drop(sender);
        worker.join().unwrap();
        server.join().unwrap();
        let trace = methods.lock().unwrap();
        assert_eq!(trace.iter().filter(|m| *m == "initialize").count(), 1);
        assert_eq!(trace.iter().filter(|m| *m == "resources/read").count(), 1);
        assert_eq!(trace.last().unwrap(), "DELETE");
    }
}

#[test]
fn credential_rotation_survives_session_recovery_and_hides_previous_tokens() {
    let (endpoint, server, methods) = fixture_mode(Mode::RotatingAuth);
    let (sender, requests) = mpsc::channel();
    let (updates, update_rx) = mpsc::channel();
    let target = endpoint.clone();
    let worker = thread::spawn(move || {
        mcp_http::serve_http_with_updates(
            HttpConfig { endpoint },
            test_credential(true),
            Arc::new(AtomicBool::new(false)),
            requests,
            update_rx,
        )
        .unwrap()
    });
    let report = ask(&sender, Action::Inspect, false).unwrap();
    let (response, ack) = mpsc::channel();
    updates
        .send(mcp_http::CredentialUpdate {
            endpoint: target,
            credential: zi_devtools::credentials::Secret::new("zi-synthetic-rotated-token".into())
                .unwrap(),
            response,
        })
        .unwrap();
    ack.recv_timeout(Duration::from_secs(3)).unwrap().unwrap();
    let read = ask(
        &sender,
        Action::ReadResource {
            uri: "fixture://guide".into(),
        },
        false,
    )
    .unwrap();
    let display = format!("{read:?}");
    assert!(!display.contains("zi-synthetic-test-token"));
    assert!(!display.contains("zi-synthetic-rotated-token"));
    let error = ask(
        &sender,
        Action::Call {
            tool: "echo".into(),
            arguments: json!({}),
            expected_tool: report.tools[0].clone(),
        },
        true,
    )
    .unwrap_err();
    assert!(error.contains("未重试"));
    let fresh = ask(&sender, Action::Inspect, false).unwrap();
    assert!(!format!("{fresh:?}").contains("zi-synthetic-test-token"));
    drop(sender);
    worker.join().unwrap();
    server.join().unwrap();
    let trace = methods.lock().unwrap();
    assert_eq!(trace.iter().filter(|m| *m == "initialize").count(), 2);
    assert_eq!(trace.iter().filter(|m| *m == "tools/call").count(), 1);
    assert_eq!(trace.last().unwrap(), "DELETE");
}

#[test]
fn rejected_credential_update_closes_session_without_using_invalid_credentials() {
    for wrong_target in [true, false] {
        let (endpoint, server, methods) = fixture_mode(Mode::Authenticated);
        let (sender, requests) = mpsc::channel();
        let (updates, update_rx) = mpsc::channel();
        let target = if wrong_target {
            format!("{endpoint}/other")
        } else {
            endpoint.clone()
        };
        let worker = thread::spawn(move || {
            mcp_http::serve_http_with_updates(
                HttpConfig { endpoint },
                test_credential(true),
                Arc::new(AtomicBool::new(false)),
                requests,
                update_rx,
            )
            .unwrap()
        });
        ask(&sender, Action::Inspect, false).unwrap();
        let (response, ack) = mpsc::channel();
        let value = if wrong_target {
            "zi-synthetic-rotated-token"
        } else {
            "synthetic-invalid bearer"
        };
        updates
            .send(mcp_http::CredentialUpdate {
                endpoint: target,
                credential: zi_devtools::credentials::Secret::new(value.into()).unwrap(),
                response,
            })
            .unwrap();
        let error = ack
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap_err();
        assert!(!error.contains(value));
        assert!(error.contains(if wrong_target {
            "不一致"
        } else {
            "格式无效"
        }));
        worker.join().unwrap();
        server.join().unwrap();
        let trace = methods.lock().unwrap();
        assert_eq!(trace.iter().filter(|m| *m == "initialize").count(), 1);
        assert_eq!(trace.iter().filter(|m| *m == "tools/list").count(), 1);
        assert_eq!(trace.last().unwrap(), "DELETE");
        assert!(ask_if_connected(&sender).is_err());
    }
}

fn ask_if_connected(sender: &mpsc::Sender<ConnectedRequest>) -> Result<(), ()> {
    let (response, _) = mpsc::channel();
    sender
        .send(ConnectedRequest {
            action: Action::Inspect,
            manual_confirmed: false,
            response,
        })
        .map_err(|_| ())
}
