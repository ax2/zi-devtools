//! Explicit public-client registration; no initial access token, persistence or replay.
use crate::{
    mcp_oauth::{self, AuthorizationMetadata},
    mcp_oauth_login::Transaction,
};
use anyhow::{Result, bail, ensure};
use reqwest::{
    Url,
    header::{ACCEPT, CONTENT_TYPE},
};
use serde::Deserialize;
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

const MAX_BYTES: usize = 64 * 1024;

/// Only validated public metadata is retained. No Debug/Serialize of server responses.
pub struct RegisteredClient {
    pub client_id: String,
    pub scopes: Vec<String>,
}

#[derive(Deserialize)]
struct Reply {
    client_id: String,
    redirect_uris: Vec<String>,
    token_endpoint_auth_method: String,
    grant_types: Vec<String>,
    response_types: Vec<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default, rename = "client_secret", deserialize_with = "present")]
    has_secret: bool,
    #[serde(default, rename = "error", deserialize_with = "present")]
    has_error: bool,
}
fn present<'de, D: serde::Deserializer<'de>>(de: D) -> std::result::Result<bool, D::Error> {
    serde::de::IgnoredAny::deserialize(de)?;
    Ok(true)
}
fn parse(bytes: &[u8], redirect: &str, scopes: &[String]) -> Result<RegisteredClient> {
    ensure!(bytes.len() <= MAX_BYTES, "注册响应超过 64 KiB");
    let reply: Reply =
        serde_json::from_slice(bytes).map_err(|_| anyhow::anyhow!("注册响应格式无效"))?;
    ensure!(
        !reply.has_secret && !reply.has_error,
        "注册响应包含不支持的凭据或错误字段"
    );
    ensure!(
        !reply.client_id.is_empty()
            && reply.client_id.len() <= 512
            && !reply.client_id.chars().any(char::is_control),
        "注册客户端 ID 无效"
    );
    ensure!(
        reply.redirect_uris == [redirect],
        "注册回调地址与本次请求不一致"
    );
    ensure!(
        reply.token_endpoint_auth_method == "none",
        "注册结果不支持公共客户端认证 none"
    );
    ensure!(
        reply.response_types == ["code"]
            && reply.grant_types.contains(&"authorization_code".into())
            && reply.grant_types.len() <= 2
            && reply
                .grant_types
                .iter()
                .all(|v| ["authorization_code", "refresh_token"].contains(&v.as_str()))
            && reply
                .grant_types
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == reply.grant_types.len(),
        "注册结果不支持本次授权码流程"
    );
    let scopes = if let Some(scope) = reply.scope {
        let granted: Vec<_> = scope.split(' ').collect();
        ensure!(
            granted.len() <= 16
                && granted
                    .iter()
                    .all(|s| !s.is_empty() && scopes.iter().any(|v| v == s))
                && granted
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    == granted.len(),
            "注册结果权限与请求不一致"
        );
        granted.into_iter().map(str::to_owned).collect()
    } else {
        ensure!(scopes.is_empty(), "注册结果未确认请求权限");
        Vec::new()
    };
    Ok(RegisteredClient {
        client_id: reply.client_id,
        scopes,
    })
}

pub fn register(
    metadata: &AuthorizationMetadata,
    resource: &str,
    redirect: &str,
    scopes: &[String],
    cancel: &AtomicBool,
) -> Result<RegisteredClient> {
    // Apply the same issuer, PKCE, public-client, redirect and scope validation as login.
    let _validation = Transaction::prepare(
        metadata,
        resource,
        "registration-validation",
        redirect,
        scopes,
    )?;
    let endpoint = mcp_oauth::secure_url(
        metadata
            .registration_endpoint
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("服务未声明客户端注册地址"))?,
        true,
    )?;
    register_at(endpoint, redirect, scopes, cancel)
}
fn register_at(
    endpoint: Url,
    redirect: &str,
    scopes: &[String],
    cancel: &AtomicBool,
) -> Result<RegisteredClient> {
    ensure!(
        !cancel.load(Ordering::Relaxed),
        "已取消客户端注册，服务端记录可能仍保留"
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| anyhow::anyhow!("无法创建注册任务"))?;
    runtime.block_on(async {
        tokio::select! {
            result = request(endpoint, redirect, scopes) => {
                ensure!(!cancel.load(Ordering::Relaxed), "已取消客户端注册，服务端记录可能仍保留");
                result
            },
            _ = async { while !cancel.load(Ordering::Relaxed) { tokio::time::sleep(Duration::from_millis(20)).await; } } => bail!("已取消客户端注册，服务端记录可能仍保留")
        }
    })
}
#[cfg(test)]
pub(crate) fn fixture_register(
    endpoint: Url,
    redirect: &str,
    scopes: &[String],
    cancel: &AtomicBool,
) -> Result<RegisteredClient> {
    ensure!(
        endpoint.scheme() == "http" && endpoint.host_str() == Some("127.0.0.1"),
        "fixture must be loopback"
    );
    register_at(endpoint, redirect, scopes, cancel)
}
async fn request(endpoint: Url, redirect: &str, scopes: &[String]) -> Result<RegisteredClient> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| anyhow::anyhow!("无法创建注册请求"))?;
    let mut payload = serde_json::json!({"client_name":"Zi DevTools", "application_type":"native", "redirect_uris":[redirect],
        "token_endpoint_auth_method":"none", "grant_types":["authorization_code", "refresh_token"], "response_types":["code"]});
    if !scopes.is_empty() {
        payload["scope"] = scopes.join(" ").into();
    }
    let mut response = client
        .post(endpoint)
        .header(ACCEPT, "application/json")
        .header(CONTENT_TYPE, "application/json")
        .body(serde_json::to_vec(&payload).map_err(|_| anyhow::anyhow!("无法构造注册请求"))?)
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("注册请求失败或超时；未自动重试，服务端记录可能仍保留"))?;
    ensure!(
        response.status().as_u16() == 201,
        "服务未接受开放注册（HTTP {}），可改用预注册客户端；未自动重试",
        response.status().as_u16()
    );
    ensure!(
        response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .eq_ignore_ascii_case("application/json")),
        "注册响应不是 JSON"
    );
    ensure!(
        response
            .content_length()
            .is_none_or(|length| length <= MAX_BYTES as u64),
        "注册响应超过 64 KiB"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow::anyhow!("注册响应读取失败或超时"))?
    {
        ensure!(
            bytes.len() + chunk.len() <= MAX_BYTES,
            "注册响应超过 64 KiB"
        );
        bytes.extend_from_slice(&chunk);
    }
    parse(&bytes, redirect, scopes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };
    const REDIRECT: &str = "http://127.0.0.1:34567/oauth/callback/zi-devtools";
    fn reply() -> serde_json::Value {
        serde_json::json!({"client_id":"synthetic-client", "redirect_uris":[REDIRECT], "token_endpoint_auth_method":"none", "grant_types":["authorization_code","refresh_token"], "response_types":["code"], "scope":"tools:read"})
    }
    fn fixture(status: &str, kind: &str, body: Vec<u8>) -> (Url, thread::JoinHandle<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = Url::parse(&format!(
            "http://{}/register",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let header = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nLocation: https://other.example.test/register\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0u8; 2048];
            loop {
                let n = stream.read(&mut buffer).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
                assert!(bytes.len() < 32768);
                if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                    let text = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                    let size: usize = text
                        .lines()
                        .find_map(|v| v.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if bytes.len() >= end + 4 + size {
                        break;
                    }
                }
            }
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(&body);
            bytes
        });
        (url, worker)
    }
    #[test]
    fn open_registration_sends_only_requested_public_metadata() {
        let (url, server) = fixture(
            "201 Created",
            "application/json",
            reply().to_string().into_bytes(),
        );
        let result = register_at(
            url,
            REDIRECT,
            &["tools:read".into()],
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(result.client_id, "synthetic-client");
        let bytes = server.join().unwrap();
        let end = bytes.windows(4).position(|v| v == b"\r\n\r\n").unwrap() + 4;
        let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
        assert!(!headers.contains("authorization:") && !headers.contains("cookie:"));
        let body: serde_json::Value = serde_json::from_slice(&bytes[end..]).unwrap();
        assert_eq!(body["redirect_uris"], serde_json::json!([REDIRECT]));
        assert_eq!(body["token_endpoint_auth_method"], "none");
        assert_eq!(body["scope"], "tools:read");
        assert_eq!(body.as_object().unwrap().len(), 7);
    }
    #[test]
    fn rejects_replaced_metadata_duplicates_secrets_and_scope_expansion_without_echo() {
        for (key, value) in [
            (
                "redirect_uris",
                serde_json::json!(["http://127.0.0.1:1/other"]),
            ),
            (
                "token_endpoint_auth_method",
                serde_json::json!("client_secret_basic"),
            ),
            ("client_secret", serde_json::Value::Null),
            ("error", serde_json::Value::Null),
            ("scope", serde_json::json!("admin")),
            (
                "grant_types",
                serde_json::json!(["authorization_code", "authorization_code"]),
            ),
            ("client_id", serde_json::json!("synthetic\ninvalid")),
        ] {
            let mut body = reply();
            body[key] = value;
            assert!(
                parse(
                    body.to_string().as_bytes(),
                    REDIRECT,
                    &["tools:read".into()]
                )
                .is_err()
            );
        }
        let duplicate = reply()
            .to_string()
            .replacen('{', "{\"client_id\":\"duplicate\",", 1);
        assert!(parse(duplicate.as_bytes(), REDIRECT, &["tools:read".into()]).is_err());
    }
    #[test]
    fn errors_redirects_media_and_byte_limits_never_echo_server_body() {
        for (status, kind, body) in [
            (
                "400 Bad Request",
                "application/json",
                b"synthetic-private-response".to_vec(),
            ),
            ("307 Temporary Redirect", "application/json", Vec::new()),
            (
                "201 Created",
                "text/plain",
                b"synthetic-private-response".to_vec(),
            ),
            ("201 Created", "application/json", vec![b'x'; MAX_BYTES + 1]),
        ] {
            let (url, server) = fixture(status, kind, body);
            let err = register_at(url, REDIRECT, &[], &AtomicBool::new(false))
                .err()
                .unwrap()
                .to_string();
            assert!(!err.contains("synthetic-private-response") && !err.contains("other.example"));
            server.join().unwrap();
        }
    }
    #[test]
    fn pre_cancel_never_connects() {
        assert!(
            register_at(
                Url::parse("http://127.0.0.1:1/register").unwrap(),
                REDIRECT,
                &[],
                &AtomicBool::new(true)
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use std::{
        io::Read,
        net::TcpListener,
        sync::{Arc, mpsc},
        thread,
    };
    #[test]
    fn cancellation_ends_an_in_flight_registration_without_replay() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = Url::parse(&format!(
            "http://{}/register",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let (seen, wait) = mpsc::channel();
        let (release, done) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut buffer = [0u8; 4096];
            assert!(socket.read(&mut buffer).unwrap() > 0);
            seen.send(()).unwrap();
            done.recv_timeout(Duration::from_secs(3)).unwrap();
        });
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let worker = thread::spawn(move || {
            register_at(
                endpoint,
                "http://127.0.0.1:34567/oauth/callback/zi-devtools",
                &[],
                &worker_cancel,
            )
            .err()
            .unwrap()
            .to_string()
        });
        wait.recv_timeout(Duration::from_secs(3)).unwrap();
        let now = std::time::Instant::now();
        cancel.store(true, Ordering::Relaxed);
        assert!(worker.join().unwrap().contains("取消"));
        assert!(now.elapsed() < Duration::from_secs(1));
        release.send(()).unwrap();
        server.join().unwrap();
    }
    #[test]
    fn public_registration_rejects_http_endpoint_before_network() {
        let metadata:AuthorizationMetadata=serde_json::from_value(serde_json::json!({"issuer":"https://auth.example.test", "authorization_endpoint":"https://auth.example.test/authorize", "token_endpoint":"https://auth.example.test/token", "registration_endpoint":"http://127.0.0.1:1/register", "response_types_supported":["code"], "code_challenge_methods_supported":["S256"], "token_endpoint_auth_methods_supported":["none"]})).unwrap();
        let error = register(
            &metadata,
            "https://mcp.example.test/mcp",
            "http://127.0.0.1:34567/oauth/callback/zi-devtools",
            &[],
            &AtomicBool::new(false),
        )
        .err()
        .unwrap()
        .to_string();
        assert!(error.contains("HTTPS"));
    }
}
