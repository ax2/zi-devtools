//! Bounded SSE/NDJSON generation with cancellation during headers and body reads.
use crate::{
    credentials::Secret,
    plugins::{Adapter, PluginTool},
};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{
    future::Future,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

#[derive(Clone, Copy)]
enum Protocol {
    OpenAi,
    Ollama,
}
struct Decoder {
    protocol: Protocol,
    line: Vec<u8>,
    event: String,
    total: usize,
    text: String,
    stopped: bool,
    done: bool,
    skip_lf: bool,
    first_line: bool,
}
impl Decoder {
    fn new(protocol: Protocol) -> Self {
        Self {
            protocol,
            line: Vec::new(),
            event: String::new(),
            total: 0,
            text: String::new(),
            stopped: false,
            done: false,
            skip_lf: false,
            first_line: true,
        }
    }
    fn feed(&mut self, bytes: &[u8]) -> Result<()> {
        self.total = self.total.saturating_add(bytes.len());
        ensure!(self.total <= 2 * 1024 * 1024, "流式响应超过 2 MiB");
        for byte in bytes {
            if self.done {
                break;
            }
            if self.skip_lf {
                self.skip_lf = false;
                if *byte == b'\n' {
                    continue;
                }
            }
            if matches!(*byte, b'\n' | b'\r') {
                self.skip_lf = *byte == b'\r';
                let bytes = std::mem::take(&mut self.line);
                let line = std::str::from_utf8(&bytes).context("流式响应不是有效 UTF-8")?;
                self.line(line)?;
            } else {
                self.line.push(*byte);
                ensure!(self.line.len() <= 256 * 1024, "流式单行超过 256 KiB");
            }
        }
        Ok(())
    }
    fn line(&mut self, line: &str) -> Result<()> {
        let line = if self.first_line {
            self.first_line = false;
            line.strip_prefix('\u{feff}').unwrap_or(line)
        } else {
            line
        };
        match self.protocol {
            Protocol::OpenAi => {
                if line.is_empty() {
                    let event = std::mem::take(&mut self.event);
                    if !event.is_empty() {
                        self.event(&event)?;
                    }
                } else if let Some(data) = line.strip_prefix("data:") {
                    if !self.event.is_empty() {
                        self.event.push('\n');
                    }
                    self.event.push_str(data.strip_prefix(' ').unwrap_or(data));
                    ensure!(self.event.len() <= 256 * 1024, "流式事件超过 256 KiB");
                }
            }
            Protocol::Ollama => {
                if !line.trim().is_empty() {
                    self.event(line)?;
                }
            }
        }
        Ok(())
    }
    fn event(&mut self, event: &str) -> Result<()> {
        if matches!(self.protocol, Protocol::OpenAi) && event.trim() == "[DONE]" {
            ensure!(self.stopped, "流已结束但缺少正常完成标记，未提交会话");
            self.done = true;
            return Ok(());
        }
        let value: Value = serde_json::from_str(event).context("流式事件不是有效 JSON")?;
        ensure!(value.get("error").is_none(), "模型服务返回流式错误");
        let piece = match self.protocol {
            Protocol::OpenAi => {
                let choices = value
                    .get("choices")
                    .and_then(Value::as_array)
                    .context("流式事件缺少 choices")?;
                ensure!(choices.len() <= 1, "暂不支持多个并行候选");
                let Some(choice) = choices.first() else {
                    return Ok(());
                };
                ensure!(
                    choice.get("index").and_then(Value::as_u64).unwrap_or(0) == 0,
                    "候选索引无效"
                );
                ensure!(
                    choice.pointer("/delta/tool_calls").is_none()
                        && choice.pointer("/delta/function_call").is_none(),
                    "暂不支持流式工具调用"
                );
                if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                    ensure!(
                        reason == "stop",
                        "模型未正常完成（截断、过滤或工具调用），未提交会话"
                    );
                    self.stopped = true;
                }
                choice.pointer("/delta/content")
            }
            Protocol::Ollama => {
                ensure!(
                    value.pointer("/message/tool_calls").is_none(),
                    "暂不支持流式工具调用"
                );
                if value.get("done").and_then(Value::as_bool) == Some(true) {
                    ensure!(
                        value
                            .get("done_reason")
                            .and_then(Value::as_str)
                            .is_none_or(|r| r == "stop"),
                        "模型未正常完成，未提交会话"
                    );
                    self.done = true;
                }
                value.pointer("/message/content")
            }
        };
        if let Some(piece) = piece.filter(|v| !v.is_null()) {
            let piece = piece.as_str().context("流式内容不是文本")?;
            ensure!(
                self.text.len().saturating_add(piece.len()) <= 256 * 1024,
                "生成文本超过 256 KiB"
            );
            self.text.push_str(piece);
        }
        Ok(())
    }
    fn finish(&mut self) -> Result<()> {
        if !self.done && !self.line.is_empty() {
            let bytes = std::mem::take(&mut self.line);
            self.line(std::str::from_utf8(&bytes).context("流末尾 UTF-8 不完整")?)?;
        }
        ensure!(self.done, "连接提前结束，部分输出未加入上下文");
        ensure!(!self.text.trim().is_empty(), "模型返回空内容");
        Ok(())
    }
}
fn visible(text: &str, secret: Option<&Secret>, finished: bool) -> String {
    let Some(secret) = secret else {
        return text.to_owned();
    };
    let token = secret.expose();
    let escaped = serde_json::to_string(token).unwrap_or_default();
    let escaped = escaped
        .get(1..escaped.len().saturating_sub(1))
        .unwrap_or(token);
    let mut patterns = vec![token];
    if escaped != token {
        patterns.push(escaped);
    }
    let mut text = text.to_owned();
    for pattern in &patterns {
        text = text.replace(pattern, "[REDACTED]");
    }
    let mut end = text.len();
    if !finished {
        // Hold a possible token prefix until the next chunk disambiguates it.
        for pattern in patterns {
            for (n, _) in pattern.char_indices().skip(1) {
                if text.ends_with(&pattern[..n]) {
                    end = end.min(text.len() - n);
                }
            }
        }
    }
    text[..end].to_owned()
}
async fn cancellable<F: Future>(future: F, cancel: &AtomicBool) -> Result<F::Output> {
    let mut future = std::pin::pin!(future);
    loop {
        ensure!(
            !cancel.load(Ordering::Relaxed),
            "已停止，部分输出未加入上下文"
        );
        if let Ok(value) = tokio::time::timeout(Duration::from_millis(50), future.as_mut()).await {
            ensure!(
                !cancel.load(Ordering::Relaxed),
                "已停止，部分输出未加入上下文"
            );
            return Ok(value);
        }
    }
}
pub fn run(
    tool: &PluginTool,
    input: &str,
    model: &str,
    secret: Option<Secret>,
    messages: Option<&[Value]>,
    cancel: &AtomicBool,
    mut update: impl FnMut(String),
) -> Result<String> {
    ensure!(
        crate::conversation::supported(tool),
        "当前模板不支持流式文本对话"
    );
    ensure!(
        !model.trim().is_empty() && model.len() <= 256,
        "请填写有效模型名称"
    );
    let Adapter::Http {
        url,
        body,
        response_pointer,
        ..
    } = &tool.adapter
    else {
        unreachable!()
    };
    let url = crate::plugins::endpoint(url)?;
    let protocol = if response_pointer.as_deref() == Some("/message/content") {
        Protocol::Ollama
    } else {
        Protocol::OpenAi
    };
    let mut body: Value =
        serde_json::from_slice(&crate::plugins::request_body(body, input, model, messages)?)?;
    body["stream"] = Value::Bool(true);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let client = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let mut request = client
            .post(url)
            .header("Content-Type", "application/json")
            .header(
                "Accept",
                match protocol {
                    Protocol::OpenAi => "text/event-stream",
                    Protocol::Ollama => "application/x-ndjson",
                },
            )
            .body(serde_json::to_vec(&body)?);
        if let Some(secret) = &secret {
            request = request.bearer_auth(secret.expose());
        }
        let mut response = cancellable(request.send(), cancel)
            .await?
            .map_err(|_| anyhow::anyhow!("流式连接失败或超时"))?;
        ensure!(
            response.status().is_success(),
            "流式接口返回 HTTP {}",
            response.status().as_u16()
        );
        let mut decoder = Decoder::new(protocol);
        let mut last_update = std::time::Instant::now();
        let result: Result<()> = async {
            loop {
                let chunk = cancellable(response.chunk(), cancel)
                    .await?
                    .map_err(|_| anyhow::anyhow!("流式读取失败或超时"))?;
                let Some(chunk) = chunk else {
                    break;
                };
                decoder.feed(&chunk)?;
                if decoder.done || last_update.elapsed() >= Duration::from_millis(100) {
                    update(visible(&decoder.text, secret.as_ref(), false));
                    last_update = std::time::Instant::now();
                }
                if decoder.done {
                    break;
                }
            }
            ensure!(
                !cancel.load(Ordering::Relaxed),
                "已停止，部分输出未加入上下文"
            );
            decoder.finish()
        }
        .await;
        // Also flush the final safe partial text on cancellation or protocol failure.
        update(visible(&decoder.text, secret.as_ref(), false));
        result?;
        Ok(visible(&decoder.text, secret.as_ref(), true))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        sync::{Arc, mpsc},
        time::Instant,
    };
    fn delta(text: &str) -> String {
        format!(
            "data: {}\r\n\r\n",
            serde_json::json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":null}]})
        )
    }
    fn ending() -> &'static str {
        "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n"
    }
    #[test]
    fn fragmented_protocols_require_completion_and_bound_output() {
        let raw = format!("{}{}", delta("你好🙂"), ending());
        let mut decoder = Decoder::new(Protocol::OpenAi);
        for byte in raw.as_bytes() {
            decoder.feed(&[*byte]).unwrap();
        }
        decoder.finish().unwrap();
        assert_eq!(decoder.text, "你好🙂");
        let mut cr = Decoder::new(Protocol::OpenAi);
        cr.feed(format!("\u{feff}{}", raw.replace("\r\n", "\r").replace('\n', "\r")).as_bytes())
            .unwrap();
        cr.finish().unwrap();
        assert_eq!(cr.text, "你好🙂");
        let mut truncated = Decoder::new(Protocol::OpenAi);
        truncated.feed(delta("partial").as_bytes()).unwrap();
        assert!(truncated.finish().is_err());
        assert!(truncated.feed(b"data: [DONE]\n\n").is_err());
        let mut limited = Decoder::new(Protocol::OpenAi);
        assert!(limited.feed(&vec![b'x'; 256 * 1024 + 1]).is_err());
        let mut ollama = Decoder::new(Protocol::Ollama);
        for byte in "{\"message\":{\"content\":\"你好\"},\"done\":false}\n{\"done\":true,\"done_reason\":\"stop\"}".as_bytes() {ollama.feed(&[*byte]).unwrap();}
        ollama.finish().unwrap();
        assert_eq!(ollama.text, "你好");
        let mut tool_call = Decoder::new(Protocol::OpenAi);
        assert!(
            tool_call
                .feed(b"data: {\"choices\":[{\"delta\":{\"tool_calls\":[]}}]}\n\n")
                .is_err()
        );
        let mut cutoff = Decoder::new(Protocol::Ollama);
        assert!(
            cutoff
                .feed(b"{\"done\":true,\"done_reason\":\"length\"}\n")
                .is_err()
        );
    }
    #[test]
    fn partial_token_prefixes_are_withheld_and_full_repetitive_tokens_redacted() {
        let secret = Secret::new("synthetic-token".into()).unwrap();
        assert_eq!(
            visible("answer synthetic-", Some(&secret), false),
            "answer "
        );
        assert_eq!(
            visible("answer synthetic-token", Some(&secret), false),
            "answer [REDACTED]"
        );
        let repeated = Secret::new("aaaa".into()).unwrap();
        assert_eq!(visible("aaaaaa", Some(&repeated), false), "[REDACTED]");
        let quoted = Secret::new("a\"bc".into()).unwrap();
        assert_eq!(visible("a\\\"bc", Some(&quoted), true), "[REDACTED]");
    }
    fn accept(listener: TcpListener) -> TcpStream {
        listener.set_nonblocking(true).unwrap();
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            match listener.accept() {
                Ok((s, _)) => {
                    s.set_nonblocking(false).unwrap();
                    s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                    return s;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < until);
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => panic!("fixture accept failed"),
            }
        }
    }
    fn request(stream: &mut TcpStream) -> Value {
        let mut data = Vec::new();
        let mut buffer = [0; 1024];
        let (start, size) = loop {
            let n = stream.read(&mut buffer).unwrap();
            assert!(n > 0);
            data.extend_from_slice(&buffer[..n]);
            assert!(data.len() < 65536);
            if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&data[..end]).to_lowercase();
                let size: usize = head
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .unwrap()
                    .trim()
                    .parse()
                    .unwrap();
                break (end + 4, size);
            }
        };
        while data.len() < start + size {
            let n = stream.read(&mut buffer).unwrap();
            assert!(n > 0);
            data.extend_from_slice(&buffer[..n]);
        }
        serde_json::from_slice(&data[start..start + size]).unwrap()
    }
    fn tool(port: u16) -> PluginTool {
        let mut manifest =
            crate::plugins::parse(include_bytes!("../plugins-examples/openai-compatible.json"))
                .unwrap();
        let mut tool = manifest.tools.remove(1);
        if let Adapter::Http { url, .. } = &mut tool.adapter {
            *url = format!("http://127.0.0.1:{port}/v1/chat/completions");
        }
        tool
    }
    #[test]
    fn localhost_stream_completes_and_cancellation_closes_pending_socket() {
        for phase in [0, 1, 2] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let tool = tool(listener.local_addr().unwrap().port());
            let cancel = Arc::new(AtomicBool::new(false));
            let (ready_tx, ready_rx) = mpsc::channel();
            let server = std::thread::spawn(move || {
                let mut socket = accept(listener);
                let body = request(&mut socket);
                assert_eq!(body["stream"], true);
                if phase == 0 {
                    let body = format!("{}{}", delta("fixture"), ending());
                    write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
                    return true;
                }
                if phase == 2 {
                    let body = delta("partial");
                    write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{body}\r\n",body.len()).unwrap();
                }
                ready_tx.send(()).unwrap();
                let mut byte = [0];
                match socket.read(&mut byte) {
                    Ok(0) => true,
                    Err(error) => {
                        matches!(
                            error.kind(),
                            std::io::ErrorKind::ConnectionReset
                                | std::io::ErrorKind::ConnectionAborted
                        )
                    }
                    Ok(_) => false,
                }
            });
            let stop = cancel.clone();
            let cancel_thread = std::thread::spawn(move || {
                if phase != 0 {
                    ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                    std::thread::sleep(Duration::from_millis(150));
                    stop.store(true, Ordering::Relaxed);
                }
            });
            let mut partial = String::new();
            let result = run(
                &tool,
                "question",
                "fixture-model",
                None,
                None,
                &cancel,
                |text| partial = text,
            );
            cancel_thread.join().unwrap();
            assert!(
                server.join().unwrap(),
                "client did not close canceled socket"
            );
            if phase == 0 {
                assert_eq!(result.unwrap(), "fixture");
            } else {
                assert!(result.unwrap_err().to_string().contains("已停止"));
            }
            if phase == 2 {
                assert_eq!(partial, "partial");
            }
        }
    }
}
