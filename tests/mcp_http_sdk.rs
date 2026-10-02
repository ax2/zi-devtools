//! Explicit opt-in interoperability against the official TypeScript SDK fixture.
use serde_json::json;
use std::{
    sync::{Arc, atomic::AtomicBool, mpsc},
    thread,
    time::Duration,
};
use zi_devtools::{
    credentials::Secret,
    mcp::{Action, ConnectedRequest, Report},
    mcp_http::{self, HttpConfig},
};

#[test]
#[ignore = "requires explicitly started official SDK fixture in auth mode"]
fn official_sdk_authentication_and_credential_rotation() {
    let endpoint = std::env::var("ZIDEVTOOLS_MCP_SDK_ENDPOINT").unwrap();
    assert!(endpoint.starts_with("http://127.0.0.1:"));
    // Exercise the SDK middleware's distinct missing, expired and scope failures.
    for (token, status) in [
        (None, "401"),
        (Some("zi-sdk-synthetic-expired"), "401"),
        (Some("zi-sdk-synthetic-restricted"), "403"),
    ] {
        let (sender, requests) = mpsc::channel();
        let worker_endpoint = endpoint.clone();
        let worker = thread::spawn(move || {
            mcp_http::serve_http_authenticated(
                HttpConfig {
                    endpoint: worker_endpoint,
                },
                token.map(|value| Secret::new(value.into()).unwrap()),
                Arc::new(AtomicBool::new(false)),
                requests,
            )
        });
        let (response, receiver) = mpsc::channel();
        sender
            .send(ConnectedRequest {
                action: Action::Inspect,
                manual_confirmed: false,
                response,
            })
            .unwrap();
        let error = receiver
            .recv_timeout(Duration::from_secs(20))
            .unwrap()
            .unwrap_err();
        assert!(error.contains(status));
        if let Some(token) = token {
            assert!(!error.contains(token));
        }
        drop(sender);
        worker.join().unwrap().unwrap();
    }

    let (sender, requests) = mpsc::channel();
    let (updates, changes) = mpsc::channel();
    let worker_endpoint = endpoint.clone();
    let worker = thread::spawn(move || {
        mcp_http::serve_http_with_updates(
            HttpConfig {
                endpoint: worker_endpoint,
            },
            Some(Secret::new("zi-sdk-synthetic-old".into()).unwrap()),
            Arc::new(AtomicBool::new(false)),
            requests,
            changes,
        )
        .unwrap()
    });
    let report = ask(&sender, Action::Inspect, false);
    assert_eq!(report.server, "Zi SDK interoperability fixture");
    let (response, receiver) = mpsc::channel();
    updates
        .send(mcp_http::CredentialUpdate {
            endpoint,
            credential: Secret::new("zi-sdk-synthetic-new".into()).unwrap(),
            response,
        })
        .unwrap();
    receiver
        .recv_timeout(Duration::from_secs(3))
        .unwrap()
        .unwrap();
    for token in ["zi-sdk-synthetic-old", "zi-sdk-synthetic-new"] {
        let result = ask(
            &sender,
            Action::Call {
                tool: "echo".into(),
                arguments: json!({"text": token}),
                expected_tool: report.tools[0].clone(),
            },
            true,
        )
        .call_result
        .unwrap();
        assert!(!result.to_string().contains(token));
    }
    let resource = ask(
        &sender,
        Action::ReadResource {
            uri: "fixture://guide".into(),
        },
        false,
    );
    assert!(resource.resource_result.is_some());
    drop(updates);
    drop(sender);
    worker.join().unwrap();
}

fn ask(sender: &mpsc::Sender<ConnectedRequest>, action: Action, manual_confirmed: bool) -> Report {
    let (response, receiver) = mpsc::channel();
    sender
        .send(ConnectedRequest {
            action,
            manual_confirmed,
            response,
        })
        .unwrap();
    receiver
        .recv_timeout(Duration::from_secs(20))
        .unwrap()
        .unwrap()
}

#[test]
#[ignore = "requires explicitly started official SDK fixture and ZIDEVTOOLS_MCP_SDK_ENDPOINT"]
fn official_sdk_lists_calls_reads_and_gets_prompts() {
    let endpoint = std::env::var("ZIDEVTOOLS_MCP_SDK_ENDPOINT")
        .expect("start the disposable SDK fixture first");
    assert!(endpoint.starts_with("http://127.0.0.1:"));
    let (sender, requests) = mpsc::channel();
    let worker = thread::spawn(move || {
        mcp_http::serve_http(
            HttpConfig { endpoint },
            Arc::new(AtomicBool::new(false)),
            requests,
        )
        .unwrap()
    });
    let report = ask(&sender, Action::Inspect, false);
    assert_eq!(report.server, "Zi SDK interoperability fixture");
    assert_eq!(report.protocol, "2025-06-18");
    assert_eq!(
        (
            report.tools.len(),
            report.resources.len(),
            report.prompts.len()
        ),
        (1, 1, 1)
    );
    let called = ask(
        &sender,
        Action::Call {
            tool: "echo".into(),
            arguments: json!({"text":"SDK synthetic echo"}),
            expected_tool: report.tools[0].clone(),
        },
        true,
    );
    assert_eq!(
        called.call_result.unwrap()["content"][0]["text"],
        "SDK synthetic echo"
    );
    let resource = ask(
        &sender,
        Action::ReadResource {
            uri: "fixture://guide".into(),
        },
        false,
    );
    assert_eq!(
        resource.resource_result.unwrap().1["contents"][0]["text"],
        "SDK synthetic guide"
    );
    let prompt = ask(
        &sender,
        Action::GetPrompt {
            name: "summary".into(),
            arguments: json!({}),
        },
        false,
    );
    assert_eq!(
        prompt.prompt_result.unwrap().1["messages"][0]["content"]["text"],
        "SDK synthetic prompt"
    );
    drop(sender);
    worker.join().unwrap();
}
