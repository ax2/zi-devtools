//! Explicit OAuth metadata reads; browser login and token operations live in separate modules.
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

#[derive(Deserialize, Clone)]
pub struct AuthorizationMetadata {
    #[serde(default)]
    pub authorization_response_iss_parameter_supported: bool,
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(default)]
    pub registration_endpoint: Option<String>,
    #[serde(default)]
    pub revocation_endpoint: Option<String>,
    #[serde(default)]
    pub revocation_endpoint_auth_methods_supported: Option<Vec<String>>,
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
    if let Some(endpoint) = &metadata.revocation_endpoint {
        secure_url(endpoint, true)?;
    }
    if let Some(methods) = &metadata.revocation_endpoint_auth_methods_supported {
        bounded_strings(methods, 16)?;
    }
    bounded_strings(&metadata.response_types_supported, 16)?;
    bounded_strings(&metadata.code_challenge_methods_supported, 16)?;
    if let Some(methods) = &metadata.token_endpoint_auth_methods_supported {
        bounded_strings(methods, 16)?;
    }
    Ok(metadata)
}

fn metadata_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .build()
        .context("无法创建 OAuth 元数据客户端")
}

fn fetch_json(url: Url) -> Result<Vec<u8>> {
    let client = metadata_client()?;
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

pub struct DiscoveryAddress {
    pub metadata_url: String,
    pub advertised: bool,
}

fn token_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&c)
}

// Split challenge fields only at commas outside quoted strings.
fn challenge_fields(header: &str) -> Result<Vec<&str>> {
    ensure!(header.len() <= 8192, "OAuth 认证提示超过 8 KiB");
    let mut fields = Vec::new();
    let (mut start, mut quoted, mut escaped) = (0, false, false);
    for (index, c) in header.bytes().enumerate() {
        ensure!(c >= 0x20 || c == b'\t', "OAuth 认证提示含控制字符");
        if escaped {
            escaped = false;
            continue;
        }
        if quoted && c == b'\\' {
            escaped = true;
            continue;
        }
        if c == b'"' {
            quoted = !quoted;
        }
        if c == b',' && !quoted {
            fields.push(header[start..index].trim());
            start = index + 1;
        }
    }
    ensure!(!quoted && !escaped, "OAuth 认证提示引号未闭合");
    fields.push(header[start..].trim());
    ensure!(fields.len() <= 32, "OAuth 认证提示参数过多");
    Ok(fields)
}

fn parameter_value(value: &str) -> Result<String> {
    if let Some(quoted) = value.strip_prefix('"') {
        ensure!(quoted.ends_with('"'), "OAuth 认证提示值无效");
        let inner = &quoted[..quoted.len() - 1];
        let mut result = String::new();
        let mut escaped = false;
        for c in inner.chars() {
            if escaped {
                result.push(c);
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else {
                ensure!(c != '"', "OAuth 认证提示值无效");
                result.push(c);
            }
        }
        ensure!(!escaped, "OAuth 认证提示转义无效");
        Ok(result)
    } else {
        ensure!(
            !value.is_empty() && value.bytes().all(token_char),
            "OAuth 认证提示值无效"
        );
        Ok(value.into())
    }
}

fn metadata_hint(headers: &reqwest::header::HeaderMap) -> Result<Option<String>> {
    let mut found = None;
    let mut total = 0;
    for (count, header) in headers
        .get_all(reqwest::header::WWW_AUTHENTICATE)
        .iter()
        .enumerate()
    {
        ensure!(count < 16, "OAuth 认证提示头过多");
        total += header.as_bytes().len();
        ensure!(total <= 8192, "OAuth 认证提示超过 8 KiB");
        let text = header.to_str().context("OAuth 认证提示不是有效文本")?;
        let mut scheme = "";
        for field in challenge_fields(text)? {
            if field.is_empty() {
                continue;
            }
            let token_end = field.bytes().take_while(|c| token_char(*c)).count();
            ensure!(token_end > 0, "OAuth 认证提示参数无效");
            let mut parameter = field;
            if !field[token_end..].trim_start().starts_with('=') {
                scheme = &field[..token_end];
                let rest = &field[token_end..];
                ensure!(
                    rest.is_empty() || rest.starts_with([' ', '\t']),
                    "OAuth 认证提示格式无效"
                );
                parameter = rest.trim();
                if parameter.is_empty() {
                    continue;
                }
            }
            let Some((key, raw)) = parameter.split_once('=') else {
                continue;
            };
            if !key.trim().eq_ignore_ascii_case("resource_metadata") {
                continue;
            }
            ensure!(
                scheme.eq_ignore_ascii_case("Bearer"),
                "当前仅支持 Bearer 授权地址提示"
            );
            ensure!(found.is_none(), "OAuth 认证提示包含多个资源配置地址");
            let url = parameter_value(raw.trim())?;
            secure_url(&url, false)?;
            found = Some(url);
        }
    }
    Ok(found)
}

fn probe_url(url: Url) -> Result<DiscoveryAddress> {
    let fallback = format!(
        "{}/.well-known/oauth-protected-resource{}",
        url.origin().ascii_serialization(),
        if url.path() == "/" { "" } else { url.path() }
    );
    // GET only, no initialization/session/tool request and no credentials.
    let response = metadata_client()?
        .get(url)
        .header("Accept", "text/event-stream")
        .send()
        .map_err(|_| anyhow::anyhow!("授权地址发现失败或超时（最长 10 秒）"))?;
    let status = response.status().as_u16();
    if status == 401
        && let Some(metadata_url) = metadata_hint(response.headers())?
    {
        return Ok(DiscoveryAddress {
            metadata_url,
            advertised: true,
        });
    }
    ensure!(
        [200, 400, 401, 404, 405].contains(&status),
        "授权地址发现返回 HTTP {status}"
    );
    Ok(DiscoveryAddress {
        metadata_url: fallback,
        advertised: false,
    })
}

pub fn discover_address(endpoint: &str) -> Result<DiscoveryAddress> {
    probe_url(secure_url(endpoint.trim(), false)?)
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
    #[test]
    fn challenge_parser_handles_schemes_quotes_and_rejects_ambiguous_targets() {
        use reqwest::header::{HeaderMap, HeaderValue, WWW_AUTHENTICATE};
        let mut headers = HeaderMap::new();
        headers.append(WWW_AUTHENTICATE, HeaderValue::from_static(r#"Basic realm="example,site", Bearer realm="mcp", RESOURCE_METADATA = "https://example.test/config,one""#));
        assert_eq!(
            metadata_hint(&headers).unwrap().unwrap(),
            "https://example.test/config,one"
        );
        headers.clear();
        headers.append(
            WWW_AUTHENTICATE,
            HeaderValue::from_static("Basic realm=site"),
        );
        headers.append(
            WWW_AUTHENTICATE,
            HeaderValue::from_static(
                r#"Bearer resource_metadata="https:\/\/example.test\/config""#,
            ),
        );
        assert_eq!(
            metadata_hint(&headers).unwrap().unwrap(),
            "https://example.test/config"
        );
        headers.append(
            WWW_AUTHENTICATE,
            HeaderValue::from_static(r#"Bearer resource_metadata="https://other.test/config""#),
        );
        assert!(metadata_hint(&headers).is_err());
        for value in [
            r#"Bearer resource_metadata="http://example.test/config""#,
            r#"Bearer resource_metadata="https://example.test/config?token=x""#,
            r#"Bearer resource_metadata="https://u:p@example.test/config""#,
            r#"Bearer resource_metadata="https://example.test/config"#,
            r#"DPoP resource_metadata="https://example.test/config""#,
        ] {
            headers.clear();
            headers.insert(WWW_AUTHENTICATE, HeaderValue::from_str(value).unwrap());
            assert!(
                metadata_hint(&headers).is_err(),
                "accepted invalid challenge"
            );
        }
    }

    #[test]
    fn discovery_reads_only_original_endpoint_and_never_follows_hint_or_redirect() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            thread,
        };
        for (status, expected) in [
            ("401 Unauthorized", Some(true)),
            ("405 Method Not Allowed", Some(false)),
            ("307 Temporary Redirect", None),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url =
                Url::parse(&format!("http://{}/mcp", listener.local_addr().unwrap())).unwrap();
            let original = url.to_string();
            let server = thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0u8; 1024];
                while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                    let n = socket.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                }
                let headers = String::from_utf8(bytes).unwrap().to_ascii_lowercase();
                assert!(headers.starts_with("get /mcp "));
                assert!(!headers.contains("authorization:"));
                assert!(!headers.contains("cookie:"));
                socket.write_all(format!("HTTP/1.1 {status}\r\nWWW-Authenticate: Bearer resource_metadata=\"https://hint.example.test/config\"\r\nLocation: https://hint.example.test/redirect\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
            });
            let result = probe_url(url); // Private HTTP fixture; production discover_address requires HTTPS.
            match expected {
                Some(true) => {
                    let value = result.unwrap();
                    assert!(value.advertised);
                    assert_eq!(value.metadata_url, "https://hint.example.test/config");
                }
                Some(false) => {
                    let value = result.unwrap();
                    assert!(!value.advertised);
                    assert_eq!(
                        value.metadata_url,
                        original.replace("/mcp", "/.well-known/oauth-protected-resource/mcp")
                    );
                }
                None => assert!(result.err().unwrap().to_string().contains("HTTP 307")),
            }
            server.join().unwrap();
        }
        assert!(discover_address("http://127.0.0.1:1/mcp").is_err());
    }
}
