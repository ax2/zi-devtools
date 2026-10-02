//! Resource-bound token exchange. No credential logging, redirects, retries or persistence.
use crate::{credentials::Secret, mcp_oauth, mcp_oauth_login::CodeGrant};
use anyhow::{Result, bail, ensure};
use reqwest::{
    Url,
    header::{ACCEPT, CONTENT_TYPE},
    redirect::Policy,
};
use serde::Deserialize;
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

const MAX_BYTES: usize = 64 * 1024;

/// Intentionally not Clone, Debug or Serialize. Only the bound resource can obtain its token.
pub struct TokenSet {
    access: Secret,
    refresh: Option<Secret>,
    expires: Option<Instant>,
    resource: String,
    issuer: String,
    client_id: String,
    scopes: Vec<String>,
    token_endpoint: Url,
}

impl TokenSet {
    pub fn access_for(&self, resource: &str) -> Result<&Secret> {
        ensure!(
            mcp_oauth::canonical_resource(resource)? == self.resource,
            "令牌不属于当前 MCP 服务"
        );
        ensure!(
            self.expires
                .is_none_or(|deadline| Instant::now() < deadline),
            "访问令牌已过期，请重新登录"
        );
        Ok(&self.access)
    }
    pub fn remaining(&self) -> Option<Duration> {
        self.expires
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
    }
    pub fn has_refresh(&self) -> bool {
        self.refresh.is_some()
    }
    pub fn bindings(&self) -> (&str, &str, &str, &[String]) {
        (&self.resource, &self.issuer, &self.client_id, &self.scopes)
    }
}

#[derive(Deserialize)]
struct TokenReply {
    access_token: String,
    token_type: String,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default, rename = "error", deserialize_with = "error_present")]
    has_error: bool,
}

fn error_present<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<bool, D::Error> {
    serde::de::IgnoredAny::deserialize(deserializer)?;
    Ok(true)
}

fn parse(bytes: &[u8], grant: &CodeGrant, started: Instant) -> Result<TokenSet> {
    let (endpoint, _, resource, client_id) = grant.bindings();
    parse_bound(
        bytes,
        TokenBinding {
            resource,
            client_id,
            issuer: grant.issuer(),
            scopes: grant.scopes(),
            endpoint,
        },
        started,
    )
}

struct TokenBinding<'a> {
    resource: &'a str,
    client_id: &'a str,
    issuer: &'a str,
    scopes: &'a [String],
    endpoint: &'a Url,
}
fn parse_bound(bytes: &[u8], binding: TokenBinding<'_>, started: Instant) -> Result<TokenSet> {
    ensure!(bytes.len() <= MAX_BYTES, "令牌响应超过 64 KiB");
    let reply: TokenReply =
        serde_json::from_slice(bytes).map_err(|_| anyhow::anyhow!("令牌响应格式无效"))?;
    ensure!(!reply.has_error, "令牌响应同时包含成功和错误字段");
    ensure!(
        reply.token_type.eq_ignore_ascii_case("Bearer"),
        "授权服务返回不支持的令牌类型"
    );
    ensure!(
        !reply.access_token.is_empty()
            && reply.access_token.len() <= 2560
            && reply
                .access_token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-._~+/=".contains(&b)),
        "访问令牌格式无效"
    );
    let scopes = if let Some(scope) = reply.scope {
        let scopes: Vec<String> = scope.split(' ').map(str::to_owned).collect();
        ensure!(
            scopes.len() <= 16
                && scopes
                    .iter()
                    .all(|value| !value.is_empty() && binding.scopes.contains(value))
                && scopes
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    == scopes.len(),
            "令牌响应权限与本次申请不一致"
        );
        scopes
    } else {
        binding.scopes.to_vec()
    };
    let expires = reply
        .expires_in
        .map(|seconds| {
            ensure!(seconds > 0, "授权服务返回已过期令牌");
            started
                .checked_add(Duration::from_secs(seconds))
                .ok_or_else(|| anyhow::anyhow!("令牌有效期超出范围"))
        })
        .transpose()?;
    ensure!(
        expires.is_none_or(|deadline| deadline > Instant::now()),
        "访问令牌在交换期间已过期"
    );
    let refresh = reply
        .refresh_token
        .map(|value| {
            ensure!(
                !value.is_empty()
                    && value.len() <= 2560
                    && value.bytes().all(|byte| (0x21..=0x7e).contains(&byte)),
                "刷新令牌格式无效"
            );
            Secret::new(value)
        })
        .transpose()?;
    Ok(TokenSet {
        access: Secret::new(reply.access_token)?,
        refresh,
        expires,
        resource: binding.resource.into(),
        issuer: binding.issuer.into(),
        client_id: binding.client_id.into(),
        scopes,
        token_endpoint: binding.endpoint.clone(),
    })
}

/// Consumes the validated grant even on failure; never retry an authorization code automatically.
pub fn exchange(grant: CodeGrant, cancel: &AtomicBool) -> Result<TokenSet> {
    let endpoint = mcp_oauth::secure_url(grant.bindings().0.as_str(), true)?;
    exchange_at(grant, endpoint, cancel)
}

fn exchange_at(grant: CodeGrant, endpoint: Url, cancel: &AtomicBool) -> Result<TokenSet> {
    ensure!(!cancel.load(Ordering::Relaxed), "已取消令牌交换");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| anyhow::anyhow!("无法创建授权请求任务"))?;
    runtime.block_on(async {
        tokio::select! {
            result = request(&grant, endpoint) => {
                ensure!(!cancel.load(Ordering::Relaxed), "已取消令牌交换");
                result
            },
            _ = cancelled(cancel) => { bail!("已取消令牌交换"); }
        }
    })
}

/// Consumes old credentials even on failure/cancellation, because rotation outcome may be unknown.
pub fn refresh(old: TokenSet, cancel: &AtomicBool) -> Result<TokenSet> {
    let endpoint = mcp_oauth::secure_url(old.token_endpoint.as_str(), true)?;
    refresh_at(old, endpoint, cancel)
}
fn refresh_at(mut old: TokenSet, endpoint: Url, cancel: &AtomicBool) -> Result<TokenSet> {
    ensure!(old.refresh.is_some(), "服务未提供刷新令牌，请重新登录");
    ensure!(
        !cancel.load(Ordering::Relaxed),
        "已取消令牌刷新，请重新登录"
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| anyhow::anyhow!("无法创建刷新任务"))?;
    let mut updated = runtime.block_on(async {
        tokio::select! {
            result = async {
                let fields = [("grant_type", "refresh_token"), ("refresh_token", old.refresh.as_ref().unwrap().expose()), ("client_id", old.client_id.as_str()), ("resource", old.resource.as_str())];
                let (bytes, started) = post(endpoint, &fields).await?;
                parse_bound(&bytes, TokenBinding { resource: &old.resource, client_id: &old.client_id, issuer: &old.issuer, scopes: &old.scopes, endpoint: &old.token_endpoint }, started)
            } => { ensure!(!cancel.load(Ordering::Relaxed), "已取消令牌刷新，请重新登录"); result },
            _ = cancelled(cancel) => { bail!("已取消令牌刷新，请重新登录"); }
        }
    })?;
    if updated.refresh.is_none() {
        updated.refresh = old.refresh.take();
    }
    Ok(updated)
}

async fn cancelled(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn request(grant: &CodeGrant, endpoint: Url) -> Result<TokenSet> {
    let (_, redirect, resource, client_id) = grant.bindings();
    let started = Instant::now();
    let fields = [
        ("grant_type", "authorization_code"),
        ("code", grant.code().expose()),
        ("code_verifier", grant.verifier().expose()),
        ("redirect_uri", redirect),
        ("resource", resource),
        ("client_id", client_id),
    ];
    let (bytes, _) = post(endpoint, &fields).await?;
    parse(&bytes, grant, started)
}

async fn post(endpoint: Url, fields: &[(&str, &str)]) -> Result<(Vec<u8>, Instant)> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| anyhow::anyhow!("无法创建令牌请求"))?;
    let started = Instant::now();
    let mut response = client
        .post(endpoint)
        .header(ACCEPT, "application/json")
        .form(fields)
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("令牌请求失败或超时，请重新登录"))?;
    if response.status().as_u16() != 200 {
        match response.status().as_u16() {
            400 | 401 | 403 => bail!("授权服务拒绝令牌交换，请检查客户端设置并重新登录"),
            300..=399 => bail!("令牌端点返回重定向，已拒绝跟随"),
            _ => bail!("授权服务返回异常状态，请重新登录"),
        }
    }
    ensure!(
        response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .eq_ignore_ascii_case("application/json")),
        "令牌响应不是 JSON"
    );
    ensure!(
        response
            .content_length()
            .is_none_or(|length| length <= MAX_BYTES as u64),
        "令牌响应超过 64 KiB"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow::anyhow!("令牌响应读取失败或超时"))?
    {
        ensure!(
            bytes.len() + chunk.len() <= MAX_BYTES,
            "令牌响应超过 64 KiB"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok((bytes, started))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{mcp_oauth::AuthorizationMetadata, mcp_oauth_login::Transaction};
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{Arc, mpsc},
        thread,
    };
    pub(super) fn grant() -> CodeGrant {
        let metadata: AuthorizationMetadata = serde_json::from_value(serde_json::json!({
            "issuer":"https://auth.example.test", "authorization_endpoint":"https://auth.example.test/authorize",
            "token_endpoint":"https://auth.example.test/token", "response_types_supported":["code"],
            "code_challenge_methods_supported":["S256"], "token_endpoint_auth_methods_supported":["none"]
        })).unwrap();
        let redirect = "http://127.0.0.1:50123/oauth/callback/synthetic-token-fixture";
        let mut tx = Transaction::prepare(
            &metadata,
            "https://mcp.example.test/mcp",
            "synthetic-client",
            redirect,
            &["tools:read".into()],
        )
        .unwrap();
        let state = tx
            .authorization_url()
            .query_pairs()
            .find(|(key, _)| key == "state")
            .unwrap()
            .1
            .into_owned();
        let mut callback = Url::parse(redirect).unwrap();
        callback
            .query_pairs_mut()
            .append_pair("state", &state)
            .append_pair("code", "synthetic-code+/=");
        tx.accept_callback(callback.as_str()).unwrap()
    }
    pub(super) fn success() -> Vec<u8> {
        br#"{"access_token":"synthetic-access","token_type":"Bearer","expires_in":3600,"refresh_token":"synthetic-refresh","scope":"tools:read"}"#.to_vec()
    }
    pub(super) fn request_bytes(stream: &mut std::net::TcpStream) -> Vec<u8> {
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut buffer = [0; 2048];
        loop {
            let size = stream.read(&mut buffer).unwrap();
            assert!(size > 0 && bytes.len() + size <= 32 * 1024);
            bytes.extend_from_slice(&buffer[..size]);
            if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]);
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .map(str::to_owned)
                    })
                    .unwrap()
                    .parse()
                    .unwrap();
                if bytes.len() >= end + 4 + length {
                    return bytes;
                }
            }
        }
    }
    pub(super) fn fixture(
        status: &str,
        content_type: &str,
        body: Vec<u8>,
    ) -> (Url, thread::JoinHandle<Vec<u8>>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let endpoint =
            Url::parse(&format!("http://{}/token", listener.local_addr().unwrap())).unwrap();
        let status = status.to_owned();
        let content_type = content_type.to_owned();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = request_bytes(&mut stream);
            let header = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nLocation: https://other.example.test/token\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(&body);
            request
        });
        (endpoint, worker)
    }
    #[test]
    fn exchange_sends_exact_bound_form_without_auth_or_cookies_and_keeps_tokens_private() {
        let (endpoint, worker) = fixture("200 OK", "application/json; charset=utf-8", success());
        let tokens = exchange_at(grant(), endpoint, &AtomicBool::new(false)).unwrap();
        let bytes = worker.join().unwrap();
        let request = String::from_utf8(bytes).unwrap();
        let (headers, body) = request.split_once("\r\n\r\n").unwrap();
        assert!(headers.starts_with("POST /token HTTP/1.1"));
        assert!(
            headers
                .to_ascii_lowercase()
                .contains("application/x-www-form-urlencoded")
        );
        assert!(
            !headers.to_ascii_lowercase().contains("authorization:")
                && !headers.to_ascii_lowercase().contains("cookie:")
        );
        let url = Url::parse(&format!("http://localhost/?{body}")).unwrap();
        let fields: std::collections::BTreeMap<_, _> = url.query_pairs().collect();
        assert_eq!(fields.len(), 6);
        assert_eq!(fields["grant_type"], "authorization_code");
        assert_eq!(fields["code"], "synthetic-code+/=");
        assert_eq!(fields["code_verifier"].len(), 43);
        assert_eq!(fields["resource"], "https://mcp.example.test/mcp");
        assert_eq!(fields["client_id"], "synthetic-client");
        assert!(fields["redirect_uri"].ends_with("/oauth/callback/synthetic-token-fixture"));
        assert!(
            tokens
                .access_for("https://mcp.example.test/mcp")
                .unwrap()
                .expose()
                == "synthetic-access"
        );
        assert!(tokens.access_for("https://other.example.test/mcp").is_err());
        assert!(tokens.remaining().unwrap() > Duration::from_secs(3500));
        assert!(tokens.has_refresh());
        assert_eq!(tokens.bindings().1, "https://auth.example.test");
        assert_eq!(tokens.bindings().3, &["tools:read"]);
    }
    #[test]
    fn malformed_ambiguous_expired_or_expanded_permission_responses_are_rejected_without_echo() {
        let grant = grant();
        for body in [
            r#"{"access_token":"synthetic-secret","token_type":"DPoP"}"#,
            r#"{"access_token":"synthetic-secret","token_type":"Bearer","expires_in":0}"#,
            r#"{"access_token":"synthetic-secret","access_token":"synthetic-second","token_type":"Bearer"}"#,
            r#"{"access_token":"synthetic-secret","token_type":"Bearer","scope":"tools:write"}"#,
            r#"{"access_token":"synthetic-secret","token_type":"Bearer","error":"synthetic-secret"}"#,
            r#"{"access_token":"synthetic-secret","token_type":"Bearer","error":null}"#,
            r#"{"access_token":"synthetic-secret","token_type":"Bearer","refresh_token":" leading-space"}"#,
            r#"{"access_token":"synthetic-secret","token_type":"Bearer","expires_in":"synthetic-secret"}"#,
            r#"{"access_token":"synthetic-secret","token_type":"Bearer","scope":"tools:read tools:read"}"#,
        ] {
            let error = parse(body.as_bytes(), &grant, Instant::now())
                .err()
                .unwrap();
            assert!(!format!("{error:#}").contains("synthetic-secret"));
        }
        assert!(parse(&vec![b'x'; MAX_BYTES + 1], &grant, Instant::now()).is_err());
        let mut tokens = parse(&success(), &grant, Instant::now()).unwrap();
        tokens.expires = Some(Instant::now() - Duration::from_secs(1));
        assert!(tokens.access_for("https://mcp.example.test/mcp").is_err());
    }
    #[test]
    fn redirects_errors_wrong_media_and_oversized_responses_never_retry_or_echo() {
        for (status, media, body, expected) in [
            (
                "307 Temporary Redirect",
                "application/json",
                b"synthetic-secret".to_vec(),
                "重定向",
            ),
            (
                "400 Bad Request",
                "application/json",
                b"synthetic-secret".to_vec(),
                "拒绝",
            ),
            ("200 OK", "text/html", b"synthetic-secret".to_vec(), "JSON"),
            (
                "200 OK",
                "application/json",
                vec![b'x'; MAX_BYTES + 1],
                "64 KiB",
            ),
        ] {
            let (endpoint, worker) = fixture(status, media, body);
            let error = exchange_at(grant(), endpoint, &AtomicBool::new(false))
                .err()
                .unwrap();
            assert!(error.to_string().contains(expected));
            assert!(!format!("{error:#}").contains("synthetic-secret"));
            worker.join().unwrap();
        }
    }
    #[test]
    fn cancellation_drops_pending_network_exchange_and_pre_cancel_sends_nothing() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint =
            Url::parse(&format!("http://{}/token", listener.local_addr().unwrap())).unwrap();
        assert!(exchange_at(grant(), endpoint, &AtomicBool::new(true)).is_err());
        assert!(listener.accept().is_err());
        listener.set_nonblocking(false).unwrap();
        let endpoint =
            Url::parse(&format!("http://{}/token", listener.local_addr().unwrap())).unwrap();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (finish_tx, finish_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            request_bytes(&mut stream);
            ready_tx.send(()).unwrap();
            finish_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        });
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let worker = thread::spawn(move || exchange_at(grant(), endpoint, &worker_cancel));
        ready_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        let start = Instant::now();
        cancel.store(true, Ordering::Relaxed);
        let error = worker.join().unwrap().err().unwrap();
        assert!(error.to_string().contains("取消"));
        assert!(start.elapsed() < Duration::from_secs(2));
        finish_tx.send(()).unwrap();
        server.join().unwrap();
    }
}

#[cfg(test)]
mod chunked_tests {
    use super::*;
    use std::{io::Write, net::TcpListener, thread};
    #[test]
    fn unknown_length_chunked_body_is_bounded_before_json_parsing() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let endpoint =
            Url::parse(&format!("http://{}/token", listener.local_addr().unwrap())).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            super::tests::request_bytes(&mut stream);
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").unwrap();
            let payload = vec![b'x'; MAX_BYTES + 1];
            let _ = stream.write_all(format!("{:x}\r\n", payload.len()).as_bytes());
            let _ = stream.write_all(&payload);
            let _ = stream.write_all(b"\r\n0\r\n\r\n");
        });
        let error = exchange_at(super::tests::grant(), endpoint, &AtomicBool::new(false))
            .err()
            .unwrap();
        assert!(error.to_string().contains("64 KiB"));
        server.join().unwrap();
    }
}

#[cfg(test)]
pub(crate) fn fixture_token(expired: bool) -> TokenSet {
    let mut token = parse(&tests::success(), &tests::grant(), Instant::now()).unwrap();
    if expired {
        token.expires = Some(Instant::now() - Duration::from_secs(1));
    }
    token
}

#[cfg(feature = "ui-preview")]
pub(crate) fn preview_token(resource: &str, issuer: &str, client_id: &str) -> TokenSet {
    TokenSet {
        access: Secret::new("synthetic-preview-access".into()).unwrap(),
        refresh: Some(Secret::new("synthetic-preview-refresh".into()).unwrap()),
        expires: Some(Instant::now() + Duration::from_secs(600)),
        resource: resource.into(),
        issuer: issuer.into(),
        client_id: client_id.into(),
        scopes: vec!["tools:read".into()],
        token_endpoint: Url::parse("https://auth.example.test/token").unwrap(),
    }
}

#[cfg(test)]
mod refresh_tests {
    use super::*;
    #[test]
    fn refresh_rotates_and_retains_only_bound_refresh_token() {
        let old = fixture_token(true);
        assert!(old.access_for("https://mcp.example.test/mcp").is_err());
        let body = br#"{"access_token":"synthetic-new-access","token_type":"Bearer","expires_in":600,"refresh_token":"synthetic-rotated-refresh","scope":"tools:read"}"#.to_vec();
        let (endpoint, server) = tests::fixture("200 OK", "application/json", body);
        let updated = refresh_at(old, endpoint, &AtomicBool::new(false)).unwrap();
        let request = String::from_utf8(server.join().unwrap()).unwrap();
        let (headers, body) = request.split_once("\r\n\r\n").unwrap();
        assert!(
            !headers.to_ascii_lowercase().contains("authorization:")
                && !headers.to_ascii_lowercase().contains("cookie:")
        );
        assert!(!request.contains("synthetic-access"));
        let url = Url::parse(&format!("http://localhost/?{body}")).unwrap();
        let fields: std::collections::BTreeMap<_, _> = url.query_pairs().collect();
        assert_eq!(fields.len(), 4);
        assert_eq!(fields["grant_type"], "refresh_token");
        assert_eq!(fields["refresh_token"], "synthetic-refresh");
        assert_eq!(fields["client_id"], "synthetic-client");
        assert_eq!(fields["resource"], "https://mcp.example.test/mcp");
        assert!(
            updated
                .access_for("https://mcp.example.test/mcp")
                .unwrap()
                .expose()
                == "synthetic-new-access"
        );
        assert_eq!(
            updated.refresh.as_ref().unwrap().expose(),
            "synthetic-rotated-refresh"
        );
        assert_eq!(
            updated.token_endpoint.as_str(),
            "https://auth.example.test/token"
        );
        let body =
            br#"{"access_token":"synthetic-next-access","token_type":"Bearer","expires_in":600}"#
                .to_vec();
        let (endpoint, server) = tests::fixture("200 OK", "application/json", body);
        let next = refresh_at(updated, endpoint, &AtomicBool::new(false)).unwrap();
        let request = String::from_utf8(server.join().unwrap()).unwrap();
        assert!(
            request.contains("synthetic-rotated-refresh")
                && !request.contains("synthetic-refresh&")
        );
        assert_eq!(
            next.refresh.as_ref().unwrap().expose(),
            "synthetic-rotated-refresh"
        );
        assert_eq!(next.bindings().3, &["tools:read"]);
    }
    #[test]
    fn missing_refresh_cancellation_and_scope_expansion_reject() {
        let mut old = fixture_token(false);
        old.refresh = None;
        assert!(refresh(old, &AtomicBool::new(false)).is_err());
        assert!(refresh(fixture_token(false), &AtomicBool::new(true)).is_err());
        let body =
            br#"{"access_token":"synthetic-secret","token_type":"Bearer","scope":"tools:write"}"#
                .to_vec();
        let (endpoint, server) = tests::fixture("200 OK", "application/json", body);
        let error = refresh_at(fixture_token(false), endpoint, &AtomicBool::new(false))
            .err()
            .unwrap();
        assert!(!format!("{error:#}").contains("synthetic-secret"));
        server.join().unwrap();
    }
    #[test]
    fn refresh_errors_and_redirects_never_echo_or_replay() {
        for status in [
            "400 Bad Request",
            "401 Unauthorized",
            "307 Temporary Redirect",
        ] {
            let (endpoint, server) = tests::fixture(
                status,
                "application/json",
                b"synthetic-refresh-secret".to_vec(),
            );
            let error = refresh_at(fixture_token(false), endpoint, &AtomicBool::new(false))
                .err()
                .unwrap();
            assert!(!format!("{error:#}").contains("synthetic-refresh-secret"));
            server.join().unwrap();
        }
    }
}
