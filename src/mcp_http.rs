//! Bounded, explicitly selected legacy Streamable HTTP client (2025 era).
//! The 2026-07-28 stateless protocol has different metadata and is not served here.
use crate::mcp::{
    self, Action, ConnectedRequest, MAX_FRAME, MAX_ITEMS, MAX_PAGES, MAX_SESSION_BYTES, Report,
};
use anyhow::{Context, Result, bail, ensure};
use reqwest::{
    Client, Url,
    header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue},
    redirect::Policy,
};
use serde_json::{Value, json};
use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, RecvTimeoutError},
    },
    time::{Duration, Instant},
};

const PROTOCOL: &str = "2025-06-18";
const ACCEPT_VALUE: &str = "application/json, text/event-stream";

fn redact_value(value: &mut Value, secret: &str) {
    match value {
        Value::String(text) => *text = text.replace(secret, "[隐藏令牌]"),
        Value::Array(items) => items.iter_mut().for_each(|item| redact_value(item, secret)),
        Value::Object(fields) => {
            let old = std::mem::take(fields);
            for (key, mut value) in old {
                redact_value(&mut value, secret);
                fields.insert(key.replace(secret, "[隐藏令牌]"), value);
            }
        }
        _ => {}
    }
}

#[derive(Debug)]
struct SessionExpired;
impl std::fmt::Display for SessionExpired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MCP HTTP 会话已失效；本次操作未自动重试")
    }
}
impl std::error::Error for SessionExpired {}

#[derive(Clone, Debug)]
pub struct HttpConfig {
    pub endpoint: String,
}

impl HttpConfig {
    pub fn validate(&self) -> Result<Url> {
        ensure!(self.endpoint.len() <= 2048, "MCP HTTP 地址超过 2048 字节");
        let url = Url::parse(self.endpoint.trim()).context("MCP HTTP 地址无效")?;
        ensure!(
            url.username().is_empty() && url.password().is_none(),
            "MCP HTTP 地址不能包含账号或密码"
        );
        ensure!(
            url.query().is_none() && url.fragment().is_none(),
            "MCP HTTP 地址不能包含查询参数或片段"
        );
        let host = url.host_str().context("MCP HTTP 地址缺少主机")?;
        ensure!(
            url.scheme() == "https"
                || (url.scheme() == "http" && (host == "127.0.0.1" || host == "[::1]")),
            "只接受 HTTPS 或 127.0.0.1/::1 本机 HTTP 端点"
        );
        Ok(url)
    }
}

struct HttpSession {
    client: Client,
    endpoint: Url,
    session_id: Option<HeaderValue>,
    protocol: String,
    server: String,
    capabilities: Value,
    next_id: u64,
    active_id: Option<u64>,
    credential: Option<Arc<crate::credentials::Secret>>,
    redaction_credentials: Vec<Arc<crate::credentials::Secret>>,
}

async fn cancelled(token: &AtomicBool) {
    while !token.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn response_value(value: &Value, id: u64) -> Result<Option<Value>> {
    ensure!(
        value.get("jsonrpc").and_then(Value::as_str) == Some("2.0"),
        "MCP HTTP 响应不是 JSON-RPC 2.0"
    );
    if value.get("method").is_some() {
        ensure!(
            value.get("id").is_none(),
            "MCP 服务发起请求，当前调试台不支持交互式回调"
        );
        return Ok(None);
    }
    ensure!(
        value.get("id").and_then(Value::as_u64) == Some(id),
        "MCP HTTP 响应 ID 与请求不匹配"
    );
    if let Some(error) = value.get("error") {
        let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("未知错误");
        bail!(
            "MCP HTTP 请求失败（{code}）：{}",
            message.chars().take(300).collect::<String>()
        );
    }
    Ok(Some(
        value
            .get("result")
            .cloned()
            .context("MCP HTTP 响应缺少 result")?,
    ))
}

impl HttpSession {
    fn create(
        config: &HttpConfig,
        credential: Option<Arc<crate::credentials::Secret>>,
    ) -> Result<Self> {
        let endpoint = config.validate()?;
        let mut headers = HeaderMap::new();
        if let Some(secret) = &credential {
            ensure!(
                secret
                    .expose()
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-._~+/=".contains(&b)),
                "Bearer 令牌格式无效"
            );
            let mut header = HeaderValue::from_str(&format!("Bearer {}", secret.expose()))
                .context("无法设置认证头")?;
            header.set_sensitive(true);
            headers.insert(AUTHORIZATION, header);
        }
        let client = crate::mcp_oauth::http_builder()
            .default_headers(headers)
            .no_proxy()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .build()
            .context("无法创建 MCP HTTP 客户端")?;
        Ok(Self {
            client,
            endpoint,
            session_id: None,
            protocol: String::new(),
            server: String::new(),
            capabilities: Value::Null,
            next_id: 1,
            active_id: None,
            redaction_credentials: credential.iter().cloned().collect(),
            credential,
        })
    }

    fn sanitize(&self, mut value: Value) -> Result<Value> {
        for secret in &self.redaction_credentials {
            if let Some(tools) = value.pointer("/result/tools").and_then(Value::as_array) {
                ensure!(
                    !tools
                        .iter()
                        .any(|tool| tool.to_string().contains(secret.expose())),
                    "MCP 工具定义包含认证内容，已拒绝使用"
                );
            }
            // Never transform routable server identifiers into a different tool/URI.
            for (field, key) in [("tools", "name"), ("resources", "uri"), ("prompts", "name")] {
                if let Some(items) = value
                    .pointer(&format!("/result/{field}"))
                    .and_then(Value::as_array)
                {
                    ensure!(
                        !items.iter().any(|item| item
                            .get(key)
                            .and_then(Value::as_str)
                            .is_some_and(|text| text.contains(secret.expose()))),
                        "MCP 服务标识包含认证内容，已拒绝使用"
                    );
                }
            }
            redact_value(&mut value, secret.expose());
        }
        Ok(value)
    }

    async fn initialize(&mut self) -> Result<()> {
        let mut budget = 0;
        let response = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL,
                    "capabilities": {},
                    "clientInfo": {"name":"zi-devtools","version":env!("CARGO_PKG_VERSION")}
                }),
                &mut budget,
            )
            .await?;
        let version = response
            .get("protocolVersion")
            .and_then(Value::as_str)
            .context("MCP HTTP 服务未返回协议版本")?;
        ensure!(
            ["2025-03-26", PROTOCOL].contains(&version),
            "MCP HTTP 服务选择不受支持的版本：{version}"
        );
        self.protocol = version.to_owned();
        self.server = response
            .pointer("/serverInfo/name")
            .and_then(Value::as_str)
            .context("MCP HTTP 服务未返回名称")?
            .chars()
            .take(120)
            .collect();
        self.capabilities = response
            .get("capabilities")
            .filter(|v| v.is_object())
            .context("MCP HTTP 服务未返回 capabilities")?
            .clone();
        self.notify_initialized().await?;
        Ok(())
    }

    async fn notify_initialized(&self) -> Result<()> {
        let payload = json!({"jsonrpc":"2.0","method":"notifications/initialized"});
        let mut response = self.post(&payload).await?;
        ensure!(
            response.status().as_u16() == 202,
            "MCP HTTP initialized 通知未返回 202"
        );
        ensure!(
            response.chunk().await?.is_none(),
            "MCP HTTP initialized 通知应返回空响应"
        );
        Ok(())
    }

    async fn post(&self, body: &Value) -> Result<reqwest::Response> {
        let payload = serde_json::to_vec(body)?;
        ensure!(payload.len() <= 256 * 1024, "MCP HTTP 请求超过 256 KiB");
        let mut request = self
            .client
            .post(self.endpoint.clone())
            .header(ACCEPT, ACCEPT_VALUE)
            .header(CONTENT_TYPE, "application/json")
            .body(payload);
        if let Some(session_id) = &self.session_id {
            request = request.header("Mcp-Session-Id", session_id);
        }
        if !self.protocol.is_empty() {
            request = request.header("MCP-Protocol-Version", &self.protocol);
        }
        let response = request.send().await.context("MCP HTTP 请求失败")?;
        ensure!(
            !response.status().is_redirection(),
            "MCP HTTP 端点发生重定向，已拒绝跟随"
        );
        match response.status().as_u16() {
            401 => bail!("MCP HTTP 认证失败（401）：请重新连接并提供有效令牌；本次操作未重试"),
            403 => bail!("MCP HTTP 访问被拒绝（403）：请检查令牌权限；本次操作未重试"),
            _ => {}
        }
        if response.status().as_u16() == 404 && self.session_id.is_some() {
            return Err(SessionExpired.into());
        }
        ensure!(
            response.status().is_success(),
            "MCP HTTP 服务返回状态 {}",
            response.status()
        );
        Ok(response)
    }

    async fn request(&mut self, method: &str, params: Value, budget: &mut usize) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        self.active_id = (method != "initialize").then_some(id);
        let body = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        let mut response = self.post(&body).await?;
        if method == "initialize" {
            if let Some(value) = response.headers().get("Mcp-Session-Id") {
                ensure!(
                    !value.as_bytes().is_empty()
                        && value.as_bytes().len() <= 512
                        && value.as_bytes().iter().all(|b| (0x21..=0x7e).contains(b)),
                    "MCP HTTP 会话 ID 无效"
                );
                self.session_id = Some(value.clone());
            }
        }
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if content_type == "application/json" {
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.context("MCP HTTP 响应读取失败")? {
                *budget = budget.saturating_add(chunk.len());
                ensure!(
                    *budget <= MAX_SESSION_BYTES,
                    "MCP HTTP 本次操作输出超过 4 MiB"
                );
                bytes.extend_from_slice(&chunk);
                ensure!(bytes.len() <= MAX_FRAME, "MCP HTTP JSON 消息超过 1 MiB");
            }
            let value: Value = serde_json::from_slice(&bytes).context("MCP HTTP 返回无效 JSON")?;
            let value = self.sanitize(value)?;
            let result = response_value(&value, id)?.context("MCP HTTP JSON 响应不是请求结果")?;
            self.active_id = None;
            return Ok(result);
        }
        ensure!(
            content_type == "text/event-stream",
            "MCP HTTP 响应类型不是 JSON 或 SSE"
        );
        let mut line = Vec::new();
        let mut data = Vec::new();
        while let Some(chunk) = response.chunk().await.context("MCP HTTP SSE 读取失败")? {
            *budget = budget.saturating_add(chunk.len());
            ensure!(
                *budget <= MAX_SESSION_BYTES,
                "MCP HTTP 本次操作输出超过 4 MiB"
            );
            for byte in chunk {
                if byte == b'\n' {
                    if line.last() == Some(&b'\r') {
                        line.pop();
                    }
                    if line.is_empty() {
                        if !data.is_empty() {
                            if data.last() == Some(&b'\n') {
                                data.pop();
                            }
                            let value: Value = serde_json::from_slice(&data)
                                .context("MCP HTTP SSE 包含无效 JSON")?;
                            let value = self.sanitize(value)?;
                            if let Some(result) = response_value(&value, id)? {
                                self.active_id = None;
                                return Ok(result);
                            }
                            data.clear();
                        }
                    } else if let Some(value) = line.strip_prefix(b"data:") {
                        let value = value.strip_prefix(b" ").unwrap_or(value);
                        data.extend_from_slice(value);
                        data.push(b'\n');
                        ensure!(data.len() <= MAX_FRAME, "MCP HTTP SSE 事件超过 1 MiB");
                    }
                    line.clear();
                } else {
                    line.push(byte);
                    ensure!(line.len() <= MAX_FRAME, "MCP HTTP SSE 行超过 1 MiB");
                }
            }
        }
        bail!("MCP HTTP SSE 在结果返回前结束")
    }

    async fn list(&mut self, method: &str, field: &str, budget: &mut usize) -> Result<Vec<Value>> {
        let mut items = Vec::new();
        let mut cursor: Option<String> = None;
        for page in 0..MAX_PAGES {
            let params = cursor
                .as_ref()
                .map_or_else(|| json!({}), |c| json!({"cursor":c}));
            let response = self.request(method, params, budget).await?;
            let page_items = response
                .get(field)
                .and_then(Value::as_array)
                .context(format!("MCP HTTP {method} 缺少 {field}"))?;
            ensure!(
                items.len() + page_items.len() <= MAX_ITEMS,
                "MCP HTTP {field} 超过 128 项"
            );
            ensure!(
                page_items.iter().all(|v| v
                    .get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.is_empty() && s.len() <= 128)),
                "MCP HTTP {field} 含无效名称"
            );
            if field == "resources" {
                ensure!(
                    page_items.iter().all(|v| v
                        .get("uri")
                        .and_then(Value::as_str)
                        .is_some_and(|s| !s.is_empty() && s.len() <= 2048)),
                    "MCP HTTP 资源 URI 无效"
                );
            }
            items.extend(page_items.iter().cloned());
            cursor = response
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if cursor.is_none() {
                return Ok(items);
            }
            ensure!(
                page + 1 < MAX_PAGES
                    && cursor
                        .as_ref()
                        .is_some_and(|s| !s.is_empty() && s.len() <= 512),
                "MCP HTTP 分页游标无效或超过 8 页"
            );
        }
        unreachable!()
    }

    async fn perform(&mut self, action: Action, manual_confirmed: bool) -> Result<Report> {
        mcp::validate_action(&action)?;
        if matches!(action, Action::Call { .. }) {
            ensure!(manual_confirmed, "MCP HTTP 工具调用须逐次人工确认");
        }
        let mut budget = 0;
        let mut report = Report {
            server: self.server.clone(),
            protocol: self.protocol.clone(),
            ..Default::default()
        };
        if self.capabilities.get("tools").is_some() {
            report.tools = self.list("tools/list", "tools", &mut budget).await?;
        }
        if self.capabilities.get("resources").is_some() {
            report.resources = self
                .list("resources/list", "resources", &mut budget)
                .await?;
        }
        if self.capabilities.get("prompts").is_some() {
            report.prompts = self.list("prompts/list", "prompts", &mut budget).await?;
        }
        match action {
            Action::Inspect => {}
            Action::Call {
                tool,
                arguments,
                expected_tool,
            } => {
                let listed = report
                    .tools
                    .iter()
                    .find(|v| v.get("name").and_then(Value::as_str) == Some(&tool))
                    .context("MCP HTTP 服务未列出此工具")?;
                ensure!(
                    *listed == expected_tool,
                    "MCP HTTP 工具定义已变化，请重新检查"
                );
                report.call_result = Some(
                    self.request(
                        "tools/call",
                        json!({"name":tool,"arguments":arguments}),
                        &mut budget,
                    )
                    .await?,
                );
            }
            Action::ReadResource { uri } => {
                ensure!(
                    report
                        .resources
                        .iter()
                        .any(|v| v.get("uri").and_then(Value::as_str) == Some(&uri)),
                    "MCP HTTP 服务未列出此资源"
                );
                let result = self
                    .request("resources/read", json!({"uri":uri}), &mut budget)
                    .await?;
                ensure!(
                    result.get("contents").and_then(Value::as_array).is_some(),
                    "MCP HTTP 资源响应缺少 contents"
                );
                report.resource_result = Some((uri, result));
            }
            Action::GetPrompt { name, arguments } => {
                ensure!(
                    report
                        .prompts
                        .iter()
                        .any(|v| v.get("name").and_then(Value::as_str) == Some(&name)),
                    "MCP HTTP 服务未列出此提示词"
                );
                let result = self
                    .request(
                        "prompts/get",
                        json!({"name":name,"arguments":arguments}),
                        &mut budget,
                    )
                    .await?;
                ensure!(
                    result.get("messages").and_then(Value::as_array).is_some(),
                    "MCP HTTP 提示词响应缺少 messages"
                );
                report.prompt_result = Some((name, result));
            }
        }
        Ok(report)
    }

    async fn cancel_active(&mut self) {
        let Some(id) = self.active_id.take() else {
            return;
        };
        let payload = json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":id,"reason":"Client stopped the operation"}});
        let _ = tokio::time::timeout(Duration::from_secs(1), self.post(&payload)).await;
    }

    async fn close(&self) {
        let Some(session_id) = &self.session_id else {
            return;
        };
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            self.client
                .delete(self.endpoint.clone())
                .header("Mcp-Session-Id", session_id)
                .header(
                    "MCP-Protocol-Version",
                    if self.protocol.is_empty() {
                        PROTOCOL
                    } else {
                        &self.protocol
                    },
                )
                .header(ACCEPT, ACCEPT_VALUE)
                .send(),
        )
        .await;
    }
}

fn bounded<T>(
    runtime: &tokio::runtime::Runtime,
    cancelled_flag: Arc<AtomicBool>,
    timeout: Duration,
    future: impl Future<Output = Result<T>>,
) -> Result<T> {
    runtime.block_on(async {
        tokio::select! {
            _ = cancelled(&cancelled_flag) => bail!("已停止 MCP HTTP 操作"),
            result = tokio::time::timeout(timeout, future) => result.context("MCP HTTP 操作超时")?,
        }
    })
}

/// Keep minted session state available for cleanup even when initialization is cancelled.
fn open_bounded(
    runtime: &tokio::runtime::Runtime,
    token: Arc<AtomicBool>,
    config: &HttpConfig,
    credential: Option<Arc<crate::credentials::Secret>>,
    redaction_credentials: &[Arc<crate::credentials::Secret>],
) -> Result<HttpSession> {
    let mut session = HttpSession::create(config, credential)?;
    session.redaction_credentials = redaction_credentials.to_vec();
    if let Err(error) = bounded(
        runtime,
        token,
        Duration::from_secs(15),
        session.initialize(),
    ) {
        runtime.block_on(session.close());
        return Err(error);
    }
    Ok(session)
}

/// Worker-thread entry point; one selected endpoint owns one logical session.
pub fn serve_http(
    config: HttpConfig,
    cancelled_flag: Arc<AtomicBool>,
    requests: Receiver<ConnectedRequest>,
) -> Result<()> {
    serve_http_authenticated(config, None, cancelled_flag, requests)
}

/// Credentials are explicit, temporary and bound to this worker's selected endpoint.
pub fn serve_http_authenticated(
    config: HttpConfig,
    credential: Option<crate::credentials::Secret>,
    cancelled_flag: Arc<AtomicBool>,
    requests: Receiver<ConnectedRequest>,
) -> Result<()> {
    let (_sender, updates) = std::sync::mpsc::channel();
    serve_http_with_updates(config, credential, cancelled_flag, requests, updates)
}

/// Acknowledged, endpoint-bound credential replacement between operations. No Debug/Clone.
pub struct CredentialUpdate {
    pub endpoint: String,
    pub credential: crate::credentials::Secret,
    pub response: std::sync::mpsc::Sender<Result<(), String>>,
}

pub fn serve_http_with_updates(
    config: HttpConfig,
    credential: Option<crate::credentials::Secret>,
    cancelled_flag: Arc<AtomicBool>,
    requests: Receiver<ConnectedRequest>,
    updates: Receiver<CredentialUpdate>,
) -> Result<()> {
    let mut credential = credential.map(Arc::new);
    // Bound retained secrets to this worker's lifetime and a fixed rotation count.
    let mut redaction_credentials: Vec<_> = credential.iter().cloned().collect();
    let first = requests.recv().context("MCP HTTP 连接尚未收到操作")?;
    if let Err(error) = config
        .validate()
        .and_then(|_| mcp::validate_action(&first.action))
    {
        let _ = first.response.send(Err(error.to_string()));
        return Ok(());
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("无法创建 MCP HTTP 运行时")?;
    let mut session = match open_bounded(
        &runtime,
        Arc::clone(&cancelled_flag),
        &config,
        credential.clone(),
        &redaction_credentials,
    ) {
        Ok(session) => session,
        Err(error) => {
            let _ = first.response.send(Err(error.to_string()));
            return Ok(());
        }
    };
    let mut pending = Some(first);
    let mut idle_since = Instant::now();
    loop {
        if cancelled_flag.load(Ordering::Relaxed) {
            break;
        }
        if let Ok(update) = updates.try_recv() {
            let result = (|| -> Result<_> {
                ensure!(
                    HttpConfig {
                        endpoint: update.endpoint
                    }
                    .validate()?
                        == session.endpoint,
                    "更新凭据目标与当前会话不一致"
                );
                ensure!(
                    redaction_credentials.len() < 32,
                    "凭据更新次数已达上限，请重新连接"
                );
                let next = Arc::new(update.credential);
                let replacement = HttpSession::create(&config, Some(next.clone()))?;
                redaction_credentials.push(next.clone());
                session.redaction_credentials = redaction_credentials.clone();
                session.client = replacement.client;
                session.credential = Some(next.clone());
                // The owner must change too, so a future 404 initialization never uses the old token.
                credential = Some(next);
                Ok(())
            })();
            let failed = result.is_err();
            let _ = update
                .response
                .send(result.map_err(|error| error.to_string()));
            if failed {
                break;
            }
            continue;
        }
        let request = match pending
            .take()
            .map(Ok)
            .unwrap_or_else(|| requests.recv_timeout(Duration::from_millis(100)))
        {
            Ok(request) => request,
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                if idle_since.elapsed() >= Duration::from_secs(120) {
                    break;
                }
                continue;
            }
        };
        let timeout = if matches!(request.action, Action::Inspect) {
            Duration::from_secs(15)
        } else {
            Duration::from_secs(30)
        };
        let result = bounded(
            &runtime,
            Arc::clone(&cancelled_flag),
            timeout,
            session.perform(request.action, request.manual_confirmed),
        );
        let expired = result
            .as_ref()
            .err()
            .is_some_and(|error| error.is::<SessionExpired>());
        if expired && !cancelled_flag.load(Ordering::Relaxed) {
            // Initialize without the obsolete ID; never replay the failed action.
            match open_bounded(
                &runtime,
                Arc::clone(&cancelled_flag),
                &config,
                credential.clone(),
                &redaction_credentials,
            ) {
                Ok(fresh) => {
                    session = fresh;
                    let _ = request.response.send(Err(
                        "MCP HTTP 会话已失效，已重新连接；本次操作未重试，请重新检查能力后再操作"
                            .into(),
                    ));
                    idle_since = Instant::now();
                    continue;
                }
                Err(error) => {
                    let _ = request
                        .response
                        .send(Err(format!("MCP HTTP 会话已失效，重新连接失败：{error}")));
                    break;
                }
            }
        }
        let failed = result.is_err();
        if failed {
            runtime.block_on(session.cancel_active());
        }
        let _ = request
            .response
            .send(result.map_err(|error| error.to_string()));
        if failed {
            break;
        }
        idle_since = Instant::now();
    }
    runtime.block_on(session.close());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadline_drops_pending_operation() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let start = Instant::now();
        let error = bounded::<()>(
            &runtime,
            Arc::new(AtomicBool::new(false)),
            Duration::from_millis(100),
            std::future::pending(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("超时"));
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn rejects_untrusted_plain_http_and_credentials() {
        for endpoint in [
            "http://example.com/mcp",
            "http://localhost:3000/mcp",
            "https://user:pass@example.com/mcp",
            "file:///etc/passwd",
            "http://127.0.0.1:3000/mcp?token=secret",
        ] {
            assert!(
                HttpConfig {
                    endpoint: endpoint.into()
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            HttpConfig {
                endpoint: "http://127.0.0.1:3000/mcp".into()
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn rejects_mismatched_or_server_initiated_responses() {
        assert!(response_value(&json!({"jsonrpc":"2.0","id":2,"result":{}}), 1).is_err());
        assert!(
            response_value(
                &json!({"jsonrpc":"2.0","id":1,"method":"sampling/createMessage"}),
                1
            )
            .is_err()
        );
    }
}
