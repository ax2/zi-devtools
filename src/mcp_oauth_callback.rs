//! Short-lived loopback callback receiver. Request URLs and headers are never logged.
use crate::mcp_oauth_login::{CodeGrant, Transaction};
use anyhow::{Context, Result, bail, ensure};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

pub struct CallbackReceiver {
    listener: TcpListener,
    redirect: String,
}
impl CallbackReceiver {
    pub fn bind() -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).context("无法创建本机登录回调")?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let path = uuid::Uuid::new_v4().simple().to_string();
        Ok(Self {
            listener,
            redirect: format!("http://127.0.0.1:{port}/oauth/callback/{path}"),
        })
    }
    pub fn redirect_uri(&self) -> &str {
        &self.redirect
    }
    pub fn receive(self, mut transaction: Transaction, cancel: &AtomicBool) -> Result<CodeGrant> {
        self.receive_until(
            &mut transaction,
            cancel,
            Instant::now() + Duration::from_secs(300),
        )
    }
    fn receive_until(
        &self,
        tx: &mut Transaction,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<CodeGrant> {
        loop {
            if cancel.load(Ordering::Relaxed) {
                tx.cancel();
                bail!("已取消本次登录");
            }
            if Instant::now() >= deadline || tx.finished() {
                tx.cancel();
                bail!("本次登录已结束或等待超过 5 分钟");
            }
            match self.listener.accept() {
                Ok((mut stream, peer)) => {
                    if !peer.ip().is_loopback() {
                        continue;
                    }
                    let result = self
                        .read_request(&mut stream, cancel, deadline)
                        .and_then(|url| tx.accept_callback(&url));
                    let (status, body) = if result.is_ok() {
                        ("200 OK", "登录回调已接收，请返回 Zi DevTools。")
                    } else {
                        (
                            "400 Bad Request",
                            "无法接受本次回调，请返回 Zi DevTools 查看登录状态。",
                        )
                    };
                    let response = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
                    let _ = stream.write_all(response.as_bytes());
                    match result {
                        Ok(grant) => return Ok(grant),
                        Err(error) if tx.finished() => return Err(error),
                        _ => {}
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(20))
                }
                Err(_) => {
                    tx.cancel();
                    bail!("本机登录回调监听失败");
                }
            }
        }
    }
    fn read_request(
        &self,
        stream: &mut TcpStream,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<String> {
        stream.set_read_timeout(Some(Duration::from_millis(100)))?;
        let request_deadline = deadline.min(Instant::now() + Duration::from_secs(1));
        let mut bytes = Vec::new();
        let mut buffer = [0; 1024];
        loop {
            ensure!(
                !cancel.load(Ordering::Relaxed) && Instant::now() < request_deadline,
                "回调请求已取消或超时"
            );
            let size = match stream.read(&mut buffer) {
                Ok(size) => size,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    continue;
                }
                Err(_) => bail!("回调请求读取失败"),
            };
            ensure!(size > 0, "回调请求不完整");
            bytes.extend_from_slice(&buffer[..size]);
            ensure!(bytes.len() <= 12 * 1024, "回调请求过长");
            if bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                break;
            }
        }
        let text = std::str::from_utf8(&bytes).context("回调请求编码无效")?;
        let mut lines = text.split("\r\n");
        let request: Vec<_> = lines.next().unwrap_or_default().split(' ').collect();
        ensure!(
            request.len() == 3 && request[0] == "GET" && request[2] == "HTTP/1.1",
            "回调请求方法无效"
        );
        let target = request[1];
        ensure!(
            target.starts_with("/oauth/callback/")
                && !target.starts_with("//")
                && target.len() <= 8192,
            "回调路径无效"
        );
        let redirect = reqwest::Url::parse(&self.redirect)?;
        let expected_host = format!("127.0.0.1:{}", redirect.port().context("回调端口无效")?);
        let mut host = None;
        for line in lines {
            if line.is_empty() {
                break;
            }
            let (name, value) = line.split_once(':').context("回调请求头无效")?;
            ensure!(
                !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
                "回调请求头无效"
            );
            if name.eq_ignore_ascii_case("host") {
                ensure!(host.is_none(), "回调 Host 重复");
                host = Some(value.trim());
            }
            ensure!(
                !name.eq_ignore_ascii_case("transfer-encoding")
                    && !name.eq_ignore_ascii_case("content-length"),
                "回调请求不得包含请求体"
            );
        }
        ensure!(host == Some(expected_host.as_str()), "回调 Host 不匹配");
        Ok(format!("http://{expected_host}{target}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp_oauth::AuthorizationMetadata;
    use std::sync::Arc;

    fn prepare(receiver: &CallbackReceiver) -> Transaction {
        let metadata: AuthorizationMetadata = serde_json::from_value(serde_json::json!({
            "issuer":"https://auth.example.test", "authorization_endpoint":"https://auth.example.test/authorize",
            "token_endpoint":"https://auth.example.test/token", "response_types_supported":["code"],
            "code_challenge_methods_supported":["S256"], "token_endpoint_auth_methods_supported":["none"],
            "authorization_response_iss_parameter_supported":true
        })).unwrap();
        Transaction::prepare(
            &metadata,
            "https://mcp.example.test/mcp",
            "synthetic-client",
            receiver.redirect_uri(),
            &[],
        )
        .unwrap()
    }
    fn send(address: &str, request: &str) -> String {
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }
    #[test]
    fn real_listener_rejects_wrong_host_method_path_state_then_accepts_one_callback() {
        let receiver = CallbackReceiver::bind().unwrap();
        let tx = prepare(&receiver);
        let redirect = reqwest::Url::parse(receiver.redirect_uri()).unwrap();
        let address = format!("127.0.0.1:{}", redirect.port().unwrap());
        let state = tx
            .authorization_url()
            .query_pairs()
            .find(|(key, _)| key == "state")
            .unwrap()
            .1
            .to_string();
        let mut url = redirect.clone();
        url.query_pairs_mut()
            .append_pair("state", &state)
            .append_pair("code", "synthetic-test-code")
            .append_pair("iss", "https://auth.example.test");
        let target = format!("{}?{}", url.path(), url.query().unwrap());
        let valid = format!("GET {target} HTTP/1.1\r\nHost: {address}\r\n\r\n");
        let cancel = AtomicBool::new(false);
        let worker = thread::spawn(move || receiver.receive(tx, &cancel));
        for request in [
            valid.replace(&format!("Host: {address}"), "Host: attacker.example.test"),
            valid.replace("GET ", "POST "),
            valid.replace(url.path(), "/oauth/callback/wrong-path"),
            valid.replace(&state, "wrong-state"),
            valid.replace("\r\n\r\n", &format!("\r\nHost: {address}\r\n\r\n")),
            valid.replace("\r\n\r\n", "\r\nContent-Length: 0\r\n\r\n"),
        ] {
            let response = send(&address, &request);
            assert!(response.starts_with("HTTP/1.1 400"));
            assert!(!response.contains(&state) && !response.contains("synthetic-test-code"));
        }
        let response = send(&address, &valid);
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("Cache-Control: no-store"));
        assert!(!response.contains(&state) && !response.contains("synthetic-test-code"));
        assert!(worker.join().unwrap().unwrap().code().expose() == "synthetic-test-code");
        assert!(TcpStream::connect(&address).is_err());
    }
    #[test]
    fn cancellation_during_incomplete_request_releases_listener_promptly() {
        let receiver = CallbackReceiver::bind().unwrap();
        let tx = prepare(&receiver);
        let address = receiver.listener.local_addr().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let worker = thread::spawn(move || receiver.receive(tx, &worker_cancel));
        let mut stalled = TcpStream::connect(address).unwrap();
        stalled.write_all(b"GET /oauth/callback/").unwrap();
        let start = Instant::now();
        cancel.store(true, Ordering::Relaxed);
        let error = worker.join().unwrap().err().unwrap();
        assert!(error.to_string().contains("取消"));
        assert!(start.elapsed() < Duration::from_secs(2));
        drop(stalled);
        assert!(TcpStream::connect(address).is_err());
    }
    #[test]
    fn callback_timeout_consumes_transaction() {
        let receiver = CallbackReceiver::bind().unwrap();
        let mut tx = prepare(&receiver);
        assert!(
            receiver
                .receive_until(
                    &mut tx,
                    &AtomicBool::new(false),
                    Instant::now() + Duration::from_millis(30)
                )
                .is_err()
        );
        assert!(tx.finished());
    }
}
