//! Explicit, bounded model-list queries; never sends a generation request.
use anyhow::{Context, Result, ensure};
use std::{
    io::Read,
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
enum Protocol {
    OpenAi,
    Ollama,
}
fn destination(source: &str) -> Result<(reqwest::Url, Protocol)> {
    let mut url = crate::plugins::endpoint(source)?;
    let path = url.path().to_owned();
    let (path, protocol) = if let Some(prefix) = path.strip_suffix("/chat/completions") {
        (format!("{prefix}/models"), Protocol::OpenAi)
    } else if let Some(prefix) = path
        .strip_suffix("/api/chat")
        .or_else(|| path.strip_suffix("/api/generate"))
    {
        (format!("{prefix}/api/tags"), Protocol::Ollama)
    } else {
        anyhow::bail!(
            "当前地址不支持自动发现，请手动填写模型；支持 /chat/completions、/api/chat 和 /api/generate"
        );
    };
    url.set_path(&path);
    Ok((url, protocol))
}
pub fn target(source: &str) -> Result<String> {
    Ok(destination(source)?.0.to_string())
}
pub struct Report {
    pub models: Vec<String>,
    pub elapsed_ms: u128,
}
fn parse(bytes: &[u8], protocol: Protocol) -> Result<Vec<String>> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).context("模型列表不是有效 JSON")?;
    let (array, field) = match protocol {
        Protocol::OpenAi => ("data", "id"),
        Protocol::Ollama => ("models", "name"),
    };
    let rows = value
        .get(array)
        .and_then(serde_json::Value::as_array)
        .context("接口响应不是受支持的模型列表")?;
    ensure!(rows.len() <= 2048, "模型条目超过 2048 个");
    let mut models = std::collections::BTreeSet::new();
    for row in rows {
        let name = row
            .get(field)
            .and_then(serde_json::Value::as_str)
            .context("模型条目缺少名称")?;
        ensure!(
            !name.trim().is_empty() && name.len() <= 256 && !name.chars().any(char::is_control),
            "模型名称为空、过长或含控制字符"
        );
        models.insert(name.to_owned());
    }
    Ok(models.into_iter().collect())
}
pub fn fetch(source: &str, temporary: String) -> Result<Report> {
    let (url, protocol) = destination(source)?;
    let secret = if temporary.trim().is_empty() {
        None
    } else {
        Some(crate::credentials::Secret::new(temporary)?)
    };
    let started = Instant::now();
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("无法创建模型查询客户端")?;
    let mut request = client.get(url).header("Accept", "application/json");
    if let Some(secret) = &secret {
        request = request.bearer_auth(secret.expose());
    }
    let response = request
        .send()
        .map_err(|_| anyhow::anyhow!("模型列表连接失败或超时（最长 10 秒）"))?;
    ensure!(
        response.status().is_success(),
        "模型列表接口返回 HTTP {}，请检查地址与认证",
        response.status().as_u16()
    );
    let mut bytes = Vec::new();
    response
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .context("读取模型列表失败")?;
    ensure!(bytes.len() <= 1024 * 1024, "模型列表响应超过 1 MiB");
    let models = parse(&bytes, protocol)?;
    // Do not display a token echoed by an untrusted endpoint as a model name.
    ensure!(
        !secret
            .as_ref()
            .is_some_and(|s| models.iter().any(|m| m.contains(s.expose()))),
        "响应包含认证信息，已丢弃模型列表"
    );
    Ok(Report {
        models,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn destinations_preserve_origin_prefix_and_validate_lists() {
        assert_eq!(
            target("https://example.com/gateway/v1/chat/completions").unwrap(),
            "https://example.com/gateway/v1/models"
        );
        assert_eq!(
            target("http://localhost:11434/api/chat").unwrap(),
            "http://localhost:11434/api/tags"
        );
        assert!(target("http://example.com/v1/chat/completions").is_err());
        assert!(target("https://example.com/v1/chat/completions?key=fixture").is_err());
        assert!(target("https://example.com/arbitrary").is_err());
        assert_eq!(
            parse(
                br#"{"data":[{"id":"b"},{"id":"a"},{"id":"b"}]}"#,
                Protocol::OpenAi
            )
            .unwrap(),
            vec!["a", "b"]
        );
        assert_eq!(
            parse(br#"{"models":[{"name":"local:latest"}]}"#, Protocol::Ollama).unwrap(),
            vec!["local:latest"]
        );
        assert!(parse(br#"{"data":[{"id":"bad\nname"}]}"#, Protocol::OpenAi).is_err());
        assert!(parse(br#"{"data":[{}]}"#, Protocol::OpenAi).is_err());
        assert!(parse(br#"{"error":"fixture"}"#, Protocol::OpenAi).is_err());
    }
    #[test]
    fn localhost_get_discovery_and_redirect_are_bounded_and_explicit() {
        use std::{io::Write, net::TcpListener};
        for (status, body, expected_ok) in [
            ("200 OK", r#"{"data":[{"id":"fixture-model"}]}"#, true),
            ("302 Found", "", false),
            ("401 Unauthorized", "fixture-secret", false),
            ("200 OK", r#"{"data":[{"id":"fixture-secret"}]}"#, false),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let source = format!(
                "http://{}/v1/chat/completions",
                listener.local_addr().unwrap()
            );
            let server = std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline);
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => panic!("fixture accept failed"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0u8; 1024];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") && request.len() < 8192 {
                    let n = stream.read(&mut buffer).unwrap();
                    if n == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..n]);
                }
                let request = String::from_utf8_lossy(&request).to_lowercase();
                let valid = request.starts_with("get /v1/models ")
                    && request.contains("authorization: bearer fixture-secret");
                write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nLocation: http://127.0.0.1:1/never-follow\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                valid
            });
            let result = fetch(&source, "fixture-secret".into());
            assert!(server.join().unwrap());
            assert_eq!(result.is_ok(), expected_ok);
            if let Err(error) = result {
                assert!(!error.to_string().contains("fixture-secret"));
            }
        }
    }
}
