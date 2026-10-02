//! Explicit OAuth metadata reads. No login, token request, credential or redirects.
use anyhow::{Context, Result, ensure};
use reqwest::Url;
use serde::Deserialize;
use std::{io::Read, time::Duration};

const MAX_BYTES: usize = 64 * 1024;

pub fn secure_url(source: &str, query: bool) -> Result<Url> {
    ensure!(
        source.len() <= 4096 && !source.chars().any(char::is_control),
        "OAuth 地址过长或含控制字符"
    );
    let url = Url::parse(source).context("OAuth 地址无效")?;
    ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
            && (query || url.query().is_none()),
        "OAuth 地址须为不含账号、密码或片段的 HTTPS 地址"
    );
    Ok(url)
}

pub fn canonical_resource(endpoint: &str) -> Result<String> {
    let url = secure_url(endpoint.trim(), false)?;
    if url.path() == "/" {
        Ok(url.as_str().trim_end_matches('/').into())
    } else {
        Ok(url.to_string())
    }
}

pub fn resource_metadata_url(endpoint: &str) -> Result<String> {
    well_known(endpoint, "oauth-protected-resource")
}

pub fn authorization_metadata_url(issuer: &str) -> Result<String> {
    well_known(issuer, "oauth-authorization-server")
}

fn well_known(source: &str, suffix: &str) -> Result<String> {
    let url = secure_url(source, false)?;
    let origin = url.origin().ascii_serialization();
    let path = if url.path() == "/" { "" } else { url.path() };
    Ok(format!("{origin}/.well-known/{suffix}{path}"))
}

#[derive(Deserialize)]
pub struct ResourceMetadata {
    pub resource: String,
    pub authorization_servers: Vec<String>,
    #[serde(default)]
    pub scopes_supported: Vec<String>,
    #[serde(default)]
    pub bearer_methods_supported: Option<Vec<String>>,
}

#[derive(Deserialize)]
pub struct AuthorizationMetadata {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(default)]
    pub registration_endpoint: Option<String>,
    pub response_types_supported: Vec<String>,
    #[serde(default)]
    pub code_challenge_methods_supported: Vec<String>,
    #[serde(default)]
    pub token_endpoint_auth_methods_supported: Option<Vec<String>>,
}

fn bounded_strings(values: &[String], count: usize) -> Result<()> {
    ensure!(
        values.len() <= count
            && values
                .iter()
                .all(|v| !v.is_empty() && v.len() <= 4096 && !v.chars().any(char::is_control)),
        "OAuth 元数据条目为空、过长或超过数量限制"
    );
    let unique: std::collections::BTreeSet<_> = values.iter().collect();
    ensure!(unique.len() == values.len(), "OAuth 元数据包含重复条目");
    Ok(())
}

fn parse_resource(bytes: &[u8], expected: &str) -> Result<ResourceMetadata> {
    ensure!(bytes.len() <= MAX_BYTES, "OAuth 元数据超过 64 KiB");
    let metadata: ResourceMetadata =
        serde_json::from_slice(bytes).context("资源授权元数据格式无效")?;
    secure_url(expected, false)?;
    secure_url(&metadata.resource, false)?;
    ensure!(
        metadata.resource == expected,
        "资源授权元数据与当前 MCP 地址不一致"
    );
    bounded_strings(&metadata.authorization_servers, 16)?;
    ensure!(
        !metadata.authorization_servers.is_empty(),
        "资源未声明授权服务器"
    );
    for issuer in &metadata.authorization_servers {
        secure_url(issuer, false)?;
    }
    bounded_strings(&metadata.scopes_supported, 64)?;
    if let Some(methods) = &metadata.bearer_methods_supported {
        bounded_strings(methods, 8)?;
        ensure!(
            methods.iter().any(|v| v == "header"),
            "资源不支持 Bearer 请求头认证"
        );
    }
    Ok(metadata)
}

fn parse_authorization(bytes: &[u8], expected: &str) -> Result<AuthorizationMetadata> {
    ensure!(bytes.len() <= MAX_BYTES, "OAuth 元数据超过 64 KiB");
    let metadata: AuthorizationMetadata =
        serde_json::from_slice(bytes).context("授权服务器元数据格式无效")?;
    secure_url(expected, false)?;
    ensure!(
        metadata.issuer == expected,
        "授权服务器 issuer 与所选目标不一致"
    );
    secure_url(&metadata.authorization_endpoint, true)?;
    secure_url(&metadata.token_endpoint, true)?;
    if let Some(endpoint) = &metadata.registration_endpoint {
        secure_url(endpoint, true)?;
    }
    bounded_strings(&metadata.response_types_supported, 16)?;
    bounded_strings(&metadata.code_challenge_methods_supported, 16)?;
    if let Some(methods) = &metadata.token_endpoint_auth_methods_supported {
        bounded_strings(methods, 16)?;
    }
    Ok(metadata)
}

fn fetch_json(url: Url) -> Result<Vec<u8>> {
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .build()
        .context("无法创建 OAuth 元数据客户端")?;
    let response = client
        .get(url)
        .header("Accept", "application/json")
        .send()
        .map_err(|_| anyhow::anyhow!("OAuth 元数据连接失败或超时（最长 10 秒）"))?;
    ensure!(
        response.status().as_u16() == 200,
        "OAuth 元数据返回 HTTP {}",
        response.status().as_u16()
    );
    let kind = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim();
    ensure!(
        kind.eq_ignore_ascii_case("application/json"),
        "OAuth 元数据不是 application/json"
    );
    let mut bytes = Vec::new();
    response
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .context("OAuth 元数据读取失败")?;
    ensure!(bytes.len() <= MAX_BYTES, "OAuth 元数据超过 64 KiB");
    Ok(bytes)
}

pub fn fetch_resource(endpoint: &str, metadata_url: &str) -> Result<ResourceMetadata> {
    let resource = canonical_resource(endpoint)?;
    let bytes = fetch_json(secure_url(metadata_url, false)?)?;
    parse_resource(&bytes, &resource)
}

pub fn fetch_authorization(issuer: &str) -> Result<AuthorizationMetadata> {
    let bytes = fetch_json(secure_url(&authorization_metadata_url(issuer)?, false)?)?;
    parse_authorization(&bytes, issuer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn preserves_resource_and_issuer_paths_and_exact_binding() {
        assert_eq!(
            resource_metadata_url("https://example.test/mcp/a").unwrap(),
            "https://example.test/.well-known/oauth-protected-resource/mcp/a"
        );
        assert_eq!(
            authorization_metadata_url("https://example.test/tenant").unwrap(),
            "https://example.test/.well-known/oauth-authorization-server/tenant"
        );
        let resource = json!({"resource":"https://example.test/mcp", "authorization_servers":["https://auth.example.test/tenant"],"scopes_supported":["read"],"bearer_methods_supported":["header"]}).to_string();
        assert!(parse_resource(resource.as_bytes(), "https://example.test/mcp").is_ok());
        assert!(parse_resource(resource.as_bytes(), "https://example.test/other").is_err());
        let auth = json!({"issuer":"https://auth.example.test/tenant", "authorization_endpoint":"https://auth.example.test/authorize","token_endpoint":"https://auth.example.test/token", "response_types_supported":["code"],"code_challenge_methods_supported":["S256"]}).to_string();
        assert!(parse_authorization(auth.as_bytes(), "https://auth.example.test/tenant").is_ok());
        assert!(parse_authorization(auth.as_bytes(), "https://auth.example.test/other").is_err());
    }
    #[test]
    fn rejects_insecure_ambiguous_and_excessive_metadata() {
        for url in [
            "http://127.0.0.1/mcp",
            "https://u:p@example.test/",
            "https://example.test/#x",
            "https://example.test/?token=x",
        ] {
            assert!(secure_url(url, false).is_err());
        }
        let repeated = json!({"resource":"https://example.test/mcp","authorization_servers":vec!["https://auth.example.test";17]}).to_string();
        assert!(parse_resource(repeated.as_bytes(), "https://example.test/mcp").is_err());
        let duplicate = br#"{"resource":"https://example.test/mcp","resource":"https://example.test/mcp","authorization_servers":["https://auth.example.test"]}"#;
        assert!(parse_resource(duplicate, "https://example.test/mcp").is_err());
        let query_only = json!({"resource":"https://example.test/mcp","authorization_servers":["https://auth.example.test"],"bearer_methods_supported":["query"]}).to_string();
        assert!(parse_resource(query_only.as_bytes(), "https://example.test/mcp").is_err());
        assert!(parse_resource(&vec![b' '; MAX_BYTES + 1], "https://example.test/mcp").is_err());
    }
    #[test]
    fn metadata_transport_has_no_credentials_and_bounds_response() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            thread,
        };
        for (status, kind, body, expected) in [
            ("200 OK", "application/json", "{}".to_owned(), None),
            (
                "307 Temporary Redirect",
                "application/json",
                "{}".to_owned(),
                Some("HTTP 307"),
            ),
            (
                "200 OK",
                "text/html",
                "{}".to_owned(),
                Some("不是 application/json"),
            ),
            (
                "200 OK",
                "application/json",
                "x".repeat(MAX_BYTES + 1),
                Some("超过 64 KiB"),
            ),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = Url::parse(&format!(
                "http://{}/metadata",
                listener.local_addr().unwrap()
            ))
            .unwrap();
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0u8; 1024];
                while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                    let n = stream.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                }
                let headers = String::from_utf8(bytes).unwrap().to_ascii_lowercase();
                assert!(headers.starts_with("get /metadata "));
                assert!(!headers.contains("authorization:"));
                assert!(!headers.contains("cookie:"));
                let header = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nLocation: http://127.0.0.1:1/other\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(header.as_bytes()).unwrap();
                let _ = stream.write_all(body.as_bytes());
            });
            let result = fetch_json(url); // Private transport fixture only; public entry points require HTTPS.
            if let Some(message) = expected {
                assert!(result.unwrap_err().to_string().contains(message));
            } else {
                assert_eq!(result.unwrap(), b"{}");
            }
            server.join().unwrap();
        }
    }
}
