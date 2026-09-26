use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use reqwest::{
    Method, Url,
    blocking::Client,
    header::{HeaderName, HeaderValue},
};
use serde::{Deserialize, Serialize};
use similar::TextDiff;

pub const HTTP_METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];

#[derive(Default)]
pub struct DiffState {
    pub diff_before: String,
    pub diff_after: String,
    pub diff_output: String,
    pub message: String,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum HttpPane {
    #[default]
    Request,
    Sites,
    History,
}

pub struct HttpTab {
    pub id: u64,
    pub name: String,
    pub method: String,
    pub url: String,
    pub headers: String,
    pub body: String,
    pub output: String,
    pub busy: bool,
    pub message: String,
}

impl HttpTab {
    fn new(id: u64) -> Self {
        Self {
            id,
            name: format!("请求 {id}"),
            method: "GET".into(),
            url: "http://127.0.0.1:8080/".into(),
            headers: String::new(),
            body: String::new(),
            output: String::new(),
            busy: false,
            message: String::new(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct HttpSite {
    pub name: String,
    pub base_url: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct HttpHistory {
    pub time: String,
    pub method: String,
    pub safe_url: String,
    pub status: String,
    pub duration_ms: u128,
}

#[derive(Default, Serialize, Deserialize)]
pub struct HttpSavedData {
    pub sites: Vec<HttpSite>,
    pub history: Vec<HttpHistory>,
}

pub struct HttpWorkbenchState {
    pub tabs: Vec<HttpTab>,
    pub selected: usize,
    pub next_id: u64,
    pub pane: HttpPane,
    pub site_name: String,
    pub site_url: String,
    pub selected_site: Option<usize>,
    pub history_filter: String,
    pub confirm_clear_history: bool,
    pub saved: HttpSavedData,
    pub message: String,
    pub storage_path: PathBuf,
}

impl HttpWorkbenchState {
    pub fn load(storage_path: PathBuf) -> Self {
        let saved = fs::read(&storage_path)
            .ok()
            .filter(|bytes| bytes.len() <= 256 * 1024)
            .and_then(|bytes| serde_json::from_slice::<HttpSavedData>(&bytes).ok())
            .unwrap_or_default();
        Self {
            tabs: vec![HttpTab::new(1)],
            selected: 0,
            next_id: 2,
            pane: HttpPane::Request,
            site_name: String::new(),
            site_url: String::new(),
            selected_site: None,
            history_filter: String::new(),
            confirm_clear_history: false,
            saved,
            message: String::new(),
            storage_path,
        }
    }

    pub fn add_tab(&mut self) -> bool {
        if self.tabs.len() >= 12 {
            self.message = "最多同时打开 12 个请求标签".into();
            return false;
        }
        self.tabs.push(HttpTab::new(self.next_id));
        self.next_id += 1;
        self.selected = self.tabs.len() - 1;
        self.pane = HttpPane::Request;
        true
    }

    pub fn close_selected(&mut self) {
        if self.tabs.len() > 1 {
            self.tabs.remove(self.selected);
            self.selected = self.selected.min(self.tabs.len() - 1);
        }
    }

    pub fn add_site(&mut self) -> Result<()> {
        let name = self.site_name.trim();
        if name.is_empty() || name.len() > 80 || self.saved.sites.len() >= 40 {
            return Err(anyhow!(
                "站点名称不能为空且最多 80 字节；最多保存 40 个站点"
            ));
        }
        let base_url = validate_site_url(&self.site_url)?;
        if self.saved.sites.iter().any(|site| site.name == name) {
            return Err(anyhow!("站点名称已存在"));
        }
        self.saved.sites.push(HttpSite {
            name: name.into(),
            base_url,
        });
        self.save()?;
        self.site_name.clear();
        self.site_url.clear();
        Ok(())
    }

    pub fn record(&mut self, method: &str, url: &str, status: &str, duration_ms: u128) {
        if let Some(safe_url) = sanitized_history_url(url) {
            self.saved.history.insert(
                0,
                HttpHistory {
                    time: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
                    method: method.into(),
                    safe_url,
                    status: status.into(),
                    duration_ms,
                },
            );
            self.saved.history.truncate(100);
            if let Err(error) = self.save() {
                self.message = format!("历史记录未能保存：{error}");
            }
        }
    }

    pub fn save(&self) -> Result<()> {
        let parent = self.storage_path.parent().context("存储路径无父目录")?;
        fs::create_dir_all(parent).context("创建 HTTP 工作台目录失败")?;
        fs::write(&self.storage_path, serde_json::to_vec_pretty(&self.saved)?)
            .context("保存 HTTP 工作台数据失败")?;
        Ok(())
    }
}

pub fn default_http_storage_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| Path::new(".").to_path_buf())
        .join(".zi-devtools")
        .join("http-workbench.json")
}

pub fn validate_site_url(value: &str) -> Result<String> {
    let url = Url::parse(value.trim()).context("站点基础地址无效")?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || value.len() > 2048
    {
        return Err(anyhow!(
            "基础地址仅支持不含凭据、查询参数和片段的 HTTP(S) URL"
        ));
    }
    Ok(url.to_string().trim_end_matches('/').to_owned())
}

pub fn sanitized_history_url(value: &str) -> Option<String> {
    let mut url = Url::parse(value.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    url.set_query(None);
    url.set_fragment(None);
    Some(url.to_string())
}

#[derive(Clone)]
pub struct HttpRequestSpec {
    pub method: String,
    pub url: String,
    pub headers: String,
    pub body: String,
}

pub fn execute_http(spec: &HttpRequestSpec) -> Result<String> {
    if spec.url.len() > 2_048 || spec.headers.len() > 8_192 || spec.body.len() > 1024 * 1024 {
        return Err(anyhow!("URL、请求头或请求体超过大小限制"));
    }
    let url = Url::parse(spec.url.trim()).context("URL 无效")?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(anyhow!("仅支持不含 URL 用户名和密码的 HTTP(S) 地址"));
    }
    let method = Method::from_bytes(spec.method.as_bytes()).context("HTTP 方法无效")?;
    if !HTTP_METHODS.contains(&method.as_str()) {
        return Err(anyhow!("不支持该 HTTP 方法"));
    }
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
        .context("无法创建 HTTP 客户端")?;
    let mut request = client.request(method, url);
    for (index, line) in spec.headers.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| anyhow!("请求头第 {} 行缺少冒号", index + 1))?;
        request = request.header(
            HeaderName::from_bytes(name.trim().as_bytes()).context("请求头名称无效")?,
            HeaderValue::from_str(value.trim()).context("请求头值无效")?,
        );
    }
    if !spec.body.is_empty() {
        request = request.body(spec.body.clone());
    }
    let started = Instant::now();
    let mut response = request.send().context("请求失败")?;
    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("未提供")
        .to_owned();
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(256 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    let truncated = bytes.len() > 256 * 1024;
    bytes.truncate(256 * 1024);
    Ok(format!(
        "HTTP {status}\n耗时 {} ms\nContent-Type: {content_type}\n重定向不会自动跟随\n\n{}{}",
        started.elapsed().as_millis(),
        String::from_utf8_lossy(&bytes),
        if truncated {
            "\n\n[响应预览已截断，仅显示前 256 KiB]"
        } else {
            ""
        }
    ))
}

pub fn compare_text(before: &str, after: &str) -> Result<String> {
    if before.len() > 128 * 1024
        || after.len() > 128 * 1024
        || before.lines().count() > 5_000
        || after.lines().count() > 5_000
    {
        return Err(anyhow!("每份文本最多 128 KiB、5000 行"));
    }
    if before == after {
        return Ok("两份文本完全相同".to_owned());
    }
    Ok(TextDiff::configure()
        .timeout(Duration::from_millis(500))
        .diff_lines(before, after)
        .unified_diff()
        .context_radius(3)
        .header("原文", "修改后")
        .to_string())
}

#[cfg(test)]
mod tests {
    use std::{io::Write, net::TcpListener, thread};

    use super::*;

    #[test]
    fn compares_lines_and_rejects_oversized_text() {
        let result = compare_text("one\ntwo\n", "one\nthree\n").unwrap();
        assert!(result.contains("-two"));
        assert!(result.contains("+three"));
        assert_eq!(compare_text("same", "same").unwrap(), "两份文本完全相同");
        assert!(compare_text(&"x".repeat(128 * 1024 + 1), "").is_err());
    }

    #[test]
    fn sends_http_request_to_local_fixture() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).unwrap();
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").unwrap();
        });
        let result = execute_http(&HttpRequestSpec {
            method: "GET".to_owned(),
            url: format!("http://127.0.0.1:{port}/health"),
            headers: String::new(),
            body: String::new(),
        })
        .unwrap();
        server.join().unwrap();
        assert!(result.contains("HTTP 200 OK"));
        assert!(result.ends_with("ok"));
        assert!(
            execute_http(&HttpRequestSpec {
                method: "GET".to_owned(),
                url: "file:///etc/passwd".to_owned(),
                headers: String::new(),
                body: String::new(),
            })
            .is_err()
        );
    }

    #[test]
    fn site_urls_and_history_keep_only_safe_metadata() {
        assert!(validate_site_url("https://example.test/api?key=value").is_err());
        assert!(validate_site_url("https://user:pass@example.test/").is_err());
        assert_eq!(
            validate_site_url("https://example.test/api/").unwrap(),
            "https://example.test/api"
        );
        assert_eq!(
            sanitized_history_url("https://example.test/api?q=private#part").unwrap(),
            "https://example.test/api"
        );
    }

    #[test]
    fn workbench_persists_sites_and_redacted_history() {
        let dir = std::env::temp_dir().join(format!("zi-http-test-{}", uuid::Uuid::new_v4()));
        let path = dir.join("workbench.json");
        let mut state = HttpWorkbenchState::load(path.clone());
        state.site_name = "Local".into();
        state.site_url = "http://127.0.0.1:8080".into();
        state.add_site().unwrap();
        state.record(
            "GET",
            "http://127.0.0.1:8080/items?secret=private",
            "HTTP 200 OK",
            3,
        );
        state.record("POST", "http://127.0.0.1:8080/items", "HTTP 201 Created", 4);
        let raw = fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("private"));
        let loaded = HttpWorkbenchState::load(path);
        assert_eq!(loaded.saved.sites.len(), 1);
        assert_eq!(loaded.saved.history.len(), 2);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn full_request_tab_limit_does_not_replace_current_draft() {
        let mut state =
            HttpWorkbenchState::load(std::env::temp_dir().join("unused-http-workbench.json"));
        for _ in 1..12 {
            assert!(state.add_tab());
        }
        state.tabs[state.selected].url = "http://127.0.0.1/draft".into();
        assert!(!state.add_tab());
        assert_eq!(state.tabs.len(), 12);
        assert_eq!(state.tabs[state.selected].url, "http://127.0.0.1/draft");
    }
}
