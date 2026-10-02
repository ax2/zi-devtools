//! Opt-in composed synthetic OAuth flow. Production HTTPS checks are unchanged.
use crate::{
    credentials::Secret,
    mcp::{Action, ConnectedRequest, Report},
    mcp_http::{self, CredentialUpdate, HttpConfig},
    mcp_oauth::AuthorizationMetadata,
    mcp_oauth_callback::CallbackReceiver,
    mcp_oauth_login::Transaction,
    mcp_oauth_registration, mcp_oauth_token,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::Url;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, atomic::AtomicBool, mpsc},
    thread,
    time::{Duration, Instant},
};

const RESOURCE: &str = "https://mcp.example.test/mcp";
const ISSUER: &str = "https://auth.example.test";

fn fields(body: &[u8]) -> BTreeMap<String, String> {
    let mut url = Url::parse("http://127.0.0.1/").unwrap();
    url.set_query(Some(std::str::from_utf8(body).unwrap()));
    url.query_pairs().into_owned().collect()
}
fn request(socket: &mut TcpStream) -> (String, Vec<u8>) {
    // Windows accepted sockets inherit the listener's nonblocking mode.
    socket.set_nonblocking(false).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut buffer = [0; 2048];
    loop {
        let n = socket.read(&mut buffer).unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&buffer[..n]);
        assert!(bytes.len() <= 32 * 1024);
        if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
            let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
            assert!(!headers.to_ascii_lowercase().contains("\r\nauthorization:"));
            assert!(!headers.to_ascii_lowercase().contains("\r\ncookie:"));
            let length: usize = headers
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                .map(|(_, value)| value.trim().parse().unwrap())
                .unwrap_or(0);
            if bytes.len() >= end + 4 + length {
                return (
                    headers.lines().next().unwrap().to_owned(),
                    bytes[end + 4..end + 4 + length].to_vec(),
                );
            }
        }
    }
}
fn fixture(sdk_endpoint: Url) -> (Url, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let handle = thread::spawn(move || {
        let mut redirect = String::new();
        let mut challenge = String::new();
        for step in 0..6 {
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "fixture request missing");
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("fixture accept failed: {error}"),
                }
            };
            let (line, body) = request(&mut socket);
            let (status, extra, reply) = match step {
                0 => {
                    assert!(line.starts_with("POST /register "));
                    let mut metadata: Value = serde_json::from_slice(&body).unwrap();
                    redirect = metadata["redirect_uris"][0].as_str().unwrap().into();
                    assert_eq!(metadata["token_endpoint_auth_method"], "none");
                    assert_eq!(metadata["scope"], "fixture:read");
                    metadata["client_id"] = "synthetic-client".into();
                    ("201 Created", String::new(), metadata.to_string())
                }
                1 => {
                    assert!(line.starts_with("GET /authorize?"));
                    let query = line
                        .split_whitespace()
                        .nth(1)
                        .unwrap()
                        .split_once('?')
                        .unwrap()
                        .1;
                    let f = fields(query.as_bytes());
                    assert_eq!(f["redirect_uri"], redirect);
                    assert_eq!(f["client_id"], "synthetic-client");
                    assert_eq!(f["resource"], RESOURCE);
                    assert_eq!(f["code_challenge_method"], "S256");
                    challenge = f["code_challenge"].clone();
                    let mut callback = Url::parse(&redirect).unwrap();
                    callback
                        .query_pairs_mut()
                        .append_pair("state", &f["state"])
                        .append_pair("iss", ISSUER)
                        .append_pair("code", "synthetic-flow-code");
                    (
                        "302 Found",
                        format!("Location: {callback}\r\n"),
                        String::new(),
                    )
                }
                2 => {
                    assert!(line.starts_with("POST /token "));
                    let f = fields(&body);
                    assert_eq!(f.len(), 6);
                    assert_eq!(f["grant_type"], "authorization_code");
                    assert_eq!(f["code"], "synthetic-flow-code");
                    assert_eq!(f["client_id"], "synthetic-client");
                    assert_eq!(f["resource"], RESOURCE);
                    assert_eq!(f["redirect_uri"], redirect);
                    assert_eq!(
                        URL_SAFE_NO_PAD.encode(Sha256::digest(f["code_verifier"].as_bytes())),
                        challenge
                    );
                    ("200 OK", String::new(), json!({"access_token":"zi-sdk-synthetic-old", "refresh_token":"synthetic-refresh-old", "token_type":"Bearer", "expires_in":120, "scope":"fixture:read"}).to_string())
                }
                3 => {
                    assert!(line.starts_with("POST /token "));
                    let f = fields(&body);
                    assert_eq!(f.len(), 4);
                    assert_eq!(f["grant_type"], "refresh_token");
                    assert_eq!(f["refresh_token"], "synthetic-refresh-old");
                    assert_eq!(f["client_id"], "synthetic-client");
                    assert_eq!(f["resource"], RESOURCE);
                    ("200 OK", String::new(), json!({"access_token":"zi-sdk-synthetic-new", "refresh_token":"synthetic-refresh-new", "token_type":"Bearer", "expires_in":120, "scope":"fixture:read"}).to_string())
                }
                4 | 5 => {
                    assert!(line.starts_with("POST /revoke "));
                    let f = fields(&body);
                    assert_eq!(f.len(), 3);
                    assert_eq!(f["client_id"], "synthetic-client");
                    assert_eq!(
                        f["token"],
                        if step == 4 {
                            "synthetic-refresh-new"
                        } else {
                            "zi-sdk-synthetic-new"
                        }
                    );
                    assert_eq!(
                        f["token_type_hint"],
                        if step == 4 {
                            "refresh_token"
                        } else {
                            "access_token"
                        }
                    );
                    let bridge = reqwest::blocking::Client::builder()
                        .no_proxy()
                        .redirect(reqwest::redirect::Policy::none())
                        .timeout(Duration::from_secs(3))
                        .build()
                        .unwrap();
                    assert_eq!(
                        bridge
                            .post(sdk_endpoint.join("/fixture/revoke").unwrap())
                            .header("Content-Type", "application/json")
                            .body(json!({"token": f["token"]}).to_string())
                            .send()
                            .unwrap()
                            .status()
                            .as_u16(),
                        200
                    );
                    ("200 OK", String::new(), String::new())
                }
                _ => unreachable!(),
            };
            write!(socket, "HTTP/1.1 {status}\r\n{extra}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).unwrap();
        }
    });
    (url, handle)
}
fn ask(sender: &mpsc::Sender<ConnectedRequest>, action: Action, confirmed: bool) -> Report {
    let (response, receiver) = mpsc::channel();
    sender
        .send(ConnectedRequest {
            action,
            manual_confirmed: confirmed,
            response,
        })
        .unwrap();
    receiver
        .recv_timeout(Duration::from_secs(20))
        .unwrap()
        .unwrap()
}

#[test]
#[ignore = "requires official SDK fixture json/sse auth lifecycle mode and ZIDEVTOOLS_MCP_SDK_ENDPOINT"]
fn registration_callback_exchange_refresh_sdk_rotation_and_revoke() {
    let endpoint = std::env::var("ZIDEVTOOLS_MCP_SDK_ENDPOINT").unwrap();
    assert!(endpoint.starts_with("http://127.0.0.1:"));
    let cancel = AtomicBool::new(false);
    let sdk_endpoint = Url::parse(&endpoint).unwrap();
    let (base, server) = fixture(sdk_endpoint.clone());
    let callback = CallbackReceiver::bind().unwrap();
    let metadata: AuthorizationMetadata = serde_json::from_value(json!({
        "issuer":ISSUER, "authorization_endpoint":format!("{ISSUER}/authorize"), "token_endpoint":format!("{ISSUER}/token"),
        "registration_endpoint":format!("{ISSUER}/register"), "revocation_endpoint":format!("{ISSUER}/revoke"),
        "revocation_endpoint_auth_methods_supported":["none"], "token_endpoint_auth_methods_supported":["none"],
        "response_types_supported":["code"], "code_challenge_methods_supported":["S256"], "authorization_response_iss_parameter_supported":true
    })).unwrap();
    let client = mcp_oauth_registration::fixture_register(
        base.join("register").unwrap(),
        callback.redirect_uri(),
        &["fixture:read".into()],
        &cancel,
    )
    .unwrap();
    let transaction = Transaction::prepare(
        &metadata,
        RESOURCE,
        &client.client_id,
        callback.redirect_uri(),
        &client.scopes,
    )
    .unwrap();
    let mut authorize = base.join("authorize").unwrap();
    authorize.set_query(transaction.authorization_url().query());
    let browser = thread::spawn(move || {
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let response = client.get(authorize).send().unwrap();
        assert_eq!(response.status().as_u16(), 302);
        let url = Url::parse(response.headers()["location"].to_str().unwrap()).unwrap();
        assert_eq!(url.host_str(), Some("127.0.0.1"));
        assert!(url.path().starts_with("/oauth/callback/"));
        assert_eq!(client.get(url).send().unwrap().status().as_u16(), 200);
    });
    let grant = callback.receive(transaction, &cancel).unwrap();
    browser.join().unwrap();
    let token =
        mcp_oauth_token::fixture_exchange(grant, base.join("token").unwrap(), &cancel).unwrap();
    assert_eq!(token.bindings().0, RESOURCE);
    let credential = Secret::new(token.access_for(RESOURCE).unwrap().expose().into()).unwrap();
    assert!(token.access_for("https://other.example.test/mcp").is_err());
    let (sender, requests) = mpsc::channel();
    let (updates, changes) = mpsc::channel();
    let worker_endpoint = endpoint.clone();
    let worker = thread::spawn(move || {
        mcp_http::serve_http_with_updates(
            HttpConfig {
                endpoint: worker_endpoint,
            },
            Some(credential),
            Arc::new(AtomicBool::new(false)),
            requests,
            changes,
        )
        .unwrap()
    });
    let report = ask(&sender, Action::Inspect, false);
    let token =
        mcp_oauth_token::fixture_refresh(token, base.join("token").unwrap(), &cancel).unwrap();
    assert_eq!(
        token.revocation_target(),
        Some("https://auth.example.test/revoke")
    );
    let (response, receiver) = mpsc::channel();
    updates
        .send(CredentialUpdate {
            endpoint: endpoint.clone(),
            credential: Secret::new(token.access_for(RESOURCE).unwrap().expose().into()).unwrap(),
            response,
        })
        .unwrap();
    receiver
        .recv_timeout(Duration::from_secs(3))
        .unwrap()
        .unwrap();
    let called = ask(
        &sender,
        Action::Call {
            tool: "echo".into(),
            arguments: json!({"text":"composed synthetic OAuth flow"}),
            expected_tool: report.tools[0].clone(),
        },
        true,
    );
    assert_eq!(
        called.call_result.unwrap()["content"][0]["text"],
        "composed synthetic OAuth flow"
    );
    drop(updates);
    drop(sender);
    worker.join().unwrap();
    mcp_oauth_token::fixture_revoke(token, base.join("revoke").unwrap(), &cancel).unwrap();
    server.join().unwrap();
    // Use the real MCP client again: SDK middleware must now deny initialization.
    let (sender, requests) = mpsc::channel();
    let worker = thread::spawn(move || {
        mcp_http::serve_http_authenticated(
            HttpConfig { endpoint },
            Some(Secret::new("zi-sdk-synthetic-new".into()).unwrap()),
            Arc::new(AtomicBool::new(false)),
            requests,
        )
        .unwrap()
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
    assert!(error.contains("401"));
    assert!(!error.contains("zi-sdk-synthetic-new"));
    drop(sender);
    worker.join().unwrap();
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    assert_eq!(
        client
            .post(sdk_endpoint.join("/fixture/finish").unwrap())
            .send()
            .unwrap()
            .status()
            .as_u16(),
        200
    );
}
