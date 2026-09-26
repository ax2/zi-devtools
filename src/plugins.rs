//! Versioned declarative extensions: no scripts, dynamic libraries or install hooks.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

pub const CATEGORIES: &[&str] = &[
    "数据与格式",
    "文本与编码",
    "网络与接口",
    "文件与系统",
    "时间与生成",
    "AI 与模型",
    "知识与检索",
    "扩展与集成",
];
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub tools: Vec<PluginTool>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginTool {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub sample: String,
    #[serde(default)]
    pub model: String,
    pub adapter: Adapter,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Adapter {
    Builtin {
        tool: String,
        #[serde(default)]
        action: usize,
        #[serde(default)]
        pattern: String,
    },
    Http {
        url: String,
        method: String,
        #[serde(default)]
        body: Value,
        #[serde(default)]
        response_pointer: Option<String>,
    },
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
pub fn endpoint(url: &str) -> Result<reqwest::Url> {
    let u = reqwest::Url::parse(url).context("接口 URL 无效")?;
    ensure!(
        u.username().is_empty()
            && u.password().is_none()
            && u.query().is_none()
            && u.fragment().is_none(),
        "接口地址不能包含账号、密码、查询参数或片段；令牌在运行页单独输入"
    );
    let local = matches!(
        u.host_str(),
        Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
    );
    ensure!(
        u.scheme() == "https" || (u.scheme() == "http" && local),
        "远程接口必须为 HTTPS；HTTP 仅允许 localhost/127.0.0.1/::1"
    );
    Ok(u)
}
pub fn parse(bytes: &[u8]) -> Result<Manifest> {
    ensure!(bytes.len() <= 256 * 1024, "插件清单最多 256 KiB");
    let m: Manifest = serde_json::from_slice(bytes).context("插件 JSON 格式或字段无效")?;
    ensure!(m.schema_version == 1, "不支持的插件协议版本");
    ensure!(
        valid_id(&m.id) && m.id != "enabled",
        "插件 ID 仅允许小写字母、数字和连字符，长度 1–64"
    );
    ensure!(
        !m.name.trim().is_empty()
            && m.name.len() <= 128
            && !m.version.is_empty()
            && m.version.len() <= 40
            && m.description.len() <= 1024,
        "插件名称、版本或说明长度无效"
    );
    ensure!(
        !m.tools.is_empty() && m.tools.len() <= 64,
        "每个插件需包含 1–64 个工具"
    );
    let mut ids = BTreeSet::new();
    for t in &m.tools {
        ensure!(valid_id(&t.id) && ids.insert(&t.id), "工具 ID 无效或重复");
        ensure!(
            !t.name.trim().is_empty()
                && t.name.len() <= 128
                && t.description.len() <= 1024
                && t.sample.len() <= 16384
                && t.model.len() <= 256,
            "工具文本过长或名称为空"
        );
        ensure!(
            CATEGORIES.contains(&t.category.as_str()),
            "工具分类不在受支持分类中"
        );
        ensure!(
            t.keywords.len() <= 32 && t.keywords.iter().all(|k| k.len() <= 80),
            "搜索关键词过多或过长"
        );
        match &t.adapter {
            Adapter::Builtin {
                tool,
                action,
                pattern,
            } => {
                let kind = crate::tools::ToolKind::ALL
                    .into_iter()
                    .find(|k| k.id() == tool)
                    .context("插件引用了未知内置工具")?;
                ensure!(
                    *action < kind.actions().len() && pattern.len() <= 4096,
                    "内置操作或参数无效"
                );
            }
            Adapter::Http {
                url,
                method,
                response_pointer,
                ..
            } => {
                endpoint(url)?;
                ensure!(
                    matches!(method.as_str(), "GET" | "POST"),
                    "仅支持 GET / POST"
                );
                ensure!(
                    response_pointer
                        .as_ref()
                        .is_none_or(|s| s.is_empty() || s.starts_with('/')),
                    "结果路径必须为 JSON Pointer"
                );
            }
        }
    }
    Ok(m)
}
#[derive(Clone)]
pub struct Installed {
    pub manifest: Manifest,
    pub digest: String,
    pub enabled: bool,
}
pub struct Store {
    pub root: PathBuf,
    pub packages: Vec<Installed>,
    pub errors: Vec<String>,
    enabled: BTreeMap<String, String>,
}
impl Store {
    pub fn load(root: PathBuf) -> Self {
        let enabled: BTreeMap<String, String> = fs::read(root.join("enabled.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let mut store = Self {
            root,
            packages: Vec::new(),
            errors: Vec::new(),
            enabled,
        };
        if let Ok(entries) = fs::read_dir(&store.root) {
            for entry in entries.flatten().take(256) {
                let path = entry.path();
                if path.extension().is_none_or(|e| e != "json")
                    || path.file_name().is_some_and(|n| n == "enabled.json")
                {
                    continue;
                }
                let read = (|| -> Result<Installed> {
                    let bytes = read_manifest(&path)?;
                    let manifest = parse(&bytes)?;
                    ensure!(
                        path.file_stem().and_then(|s| s.to_str()) == Some(&manifest.id),
                        "插件文件名与 ID 不一致"
                    );
                    let digest = format!("{:x}", Sha256::digest(&bytes));
                    let enabled = store.enabled.get(&manifest.id) == Some(&digest);
                    Ok(Installed {
                        manifest,
                        digest,
                        enabled,
                    })
                })();
                match read {
                    Ok(p) => store.packages.push(p),
                    Err(e) => store.errors.push(format!(
                        "{}：{e}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    )),
                }
            }
        }
        store
            .packages
            .sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
        store
    }
    pub fn install(&mut self, bytes: &[u8]) -> Result<()> {
        let m = parse(bytes)?;
        ensure!(self.packages.len() < 128, "最多安装 128 个插件");
        fs::create_dir_all(&self.root)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.root.join(format!("{}.json", m.id)))
            .context("同 ID 插件已存在；请先卸载旧版")?;
        use std::io::Write;
        file.write_all(bytes)?;
        file.sync_all()?;
        *self = Self::load(self.root.clone());
        Ok(())
    }
    pub fn set_enabled(&mut self, id: &str, on: bool) -> Result<()> {
        let package = self
            .packages
            .iter()
            .find(|p| p.manifest.id == id)
            .context("插件不存在")?;
        let mut next = self.enabled.clone();
        if on {
            next.insert(id.into(), package.digest.clone());
        } else {
            next.remove(id);
        }
        fs::write(
            self.root.join("enabled.json"),
            serde_json::to_vec_pretty(&next)?,
        )?;
        *self = Self::load(self.root.clone());
        Ok(())
    }
    pub fn uninstall(&mut self, id: &str) -> Result<()> {
        ensure!(valid_id(id), "插件 ID 无效");
        self.set_enabled(id, false)?;
        fs::remove_file(self.root.join(format!("{id}.json")))?;
        *self = Self::load(self.root.clone());
        Ok(())
    }
    pub fn tools(&self) -> Vec<(String, PluginTool)> {
        self.packages
            .iter()
            .filter(|p| p.enabled)
            .flat_map(|p| {
                p.manifest
                    .tools
                    .iter()
                    .map(|t| (format!("plugin:{}/{}", p.manifest.id, t.id), t.clone()))
            })
            .collect()
    }
}
pub fn read_manifest(path: &Path) -> Result<Vec<u8>> {
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_file() && !meta.file_type().is_symlink() && meta.len() <= 256 * 1024,
        "请选择最多 256 KiB 的普通 JSON 文件"
    );
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(256 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 256 * 1024, "插件清单最多 256 KiB");
    Ok(bytes)
}
fn expand(value: &Value, input: &str, model: &str) -> Result<Value> {
    fn budget(v: &Value, input: &str, model: &str, remaining: &mut usize) -> Result<()> {
        let size = match v {
            Value::String(s) if s == "$input" || s == "$json" => input.len().saturating_mul(6),
            Value::String(s) if s == "$model" => model.len().saturating_mul(6),
            Value::String(s) => s.len().saturating_mul(6),
            Value::Object(o) => o.keys().map(|k| k.len().saturating_mul(6) + 4).sum(),
            _ => 16,
        };
        *remaining = remaining
            .checked_sub(size)
            .context("模板展开后的请求可能超过 2 MiB，请缩小输入")?;
        match v {
            Value::Array(a) => {
                for v in a {
                    budget(v, input, model, remaining)?;
                }
            }
            Value::Object(o) => {
                for v in o.values() {
                    budget(v, input, model, remaining)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    budget(value, input, model, &mut (2 * 1024 * 1024))?;
    expand_value(value, input, model)
}
fn expand_value(value: &Value, input: &str, model: &str) -> Result<Value> {
    Ok(match value {
        Value::String(s) if s == "$input" => Value::String(input.into()),
        Value::String(s) if s == "$model" => Value::String(model.into()),
        Value::String(s) if s == "$json" => {
            serde_json::from_str(input).context("此插件需要 JSON 输入")?
        }
        Value::Array(a) => Value::Array(
            a.iter()
                .map(|v| expand_value(v, input, model))
                .collect::<Result<_>>()?,
        ),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, v)| Ok((k.clone(), expand_value(v, input, model)?)))
                .collect::<Result<_>>()?,
        ),
        _ => value.clone(),
    })
}
pub fn execute(tool: &PluginTool, input: &str, model: &str, token: &str) -> Result<String> {
    crate::tools_extra::bounded(input)?;
    ensure!(
        model.len() <= 256 && token.len() <= 8192,
        "模型名称或令牌过长"
    );
    match &tool.adapter {
        Adapter::Builtin {
            tool,
            action,
            pattern,
        } => {
            let kind = crate::tools::ToolKind::ALL
                .into_iter()
                .find(|k| k.id() == tool)
                .context("内置工具不存在")?;
            crate::tools::run_tool(kind, *action, input, pattern, 10)
        }
        Adapter::Http {
            url,
            method,
            body,
            response_pointer,
        } => {
            let url = endpoint(url)?;
            let client = reqwest::blocking::Client::builder()
                .no_proxy()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .build()?;
            let mut request = client
                .request(
                    if method == "GET" {
                        reqwest::Method::GET
                    } else {
                        reqwest::Method::POST
                    },
                    url,
                )
                .header("Accept", "application/json");
            if !token.trim().is_empty() {
                request = request.bearer_auth(token.trim());
            }
            if method == "POST" {
                let body = serde_json::to_vec(&expand(body, input, model)?)?;
                ensure!(body.len() <= 2 * 1024 * 1024, "请求体超过 2 MiB");
                request = request
                    .header("Content-Type", "application/json")
                    .body(body);
            }
            let response = request.send().context("连接失败或请求超时")?;
            let status = response.status();
            ensure!(
                status.is_success(),
                "接口返回 HTTP {}（不跟随重定向）",
                status.as_u16()
            );
            let mut bytes = Vec::new();
            response.take(2 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
            ensure!(bytes.len() <= 2 * 1024 * 1024, "响应超过 2 MiB");
            let value: Value =
                serde_json::from_slice(&bytes).context("接口未返回 JSON；暂不支持 SSE/流式响应")?;
            let value = if let Some(pointer) = response_pointer {
                value.pointer(pointer).context("响应缺少插件指定字段")?
            } else {
                &value
            };
            Ok(if let Some(s) = value.as_str() {
                s.into()
            } else {
                serde_json::to_string_pretty(value)?
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const DEMO: &[u8] = include_bytes!("../plugins-examples/local-text.json");
    #[test]
    fn install_enable_tamper_disable_and_remove() {
        let root = std::env::temp_dir().join(format!("zi-plugins-{}", uuid::Uuid::new_v4()));
        let mut store = Store::load(root.clone());
        store.install(DEMO).unwrap();
        assert!(store.tools().is_empty());
        assert!(store.install(DEMO).is_err());
        store.set_enabled("local-text", true).unwrap();
        assert_eq!(store.tools().len(), 2);
        let mut bytes = DEMO.to_vec();
        bytes.push(b' ');
        fs::write(root.join("local-text.json"), bytes).unwrap();
        store = Store::load(root.clone());
        assert!(store.tools().is_empty());
        store.uninstall("local-text").unwrap();
        assert!(store.packages.is_empty());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn validates_manifest_and_endpoint_boundaries() {
        let s = String::from_utf8(DEMO.to_vec()).unwrap();
        assert!(parse(s.replace("local-text", "../escape").as_bytes()).is_err());
        for url in [
            "file:///secret",
            "http://example.com/api",
            "https://user:pass@example.com",
            "https://example.com?token=x",
        ] {
            assert!(endpoint(url).is_err());
        }
        assert!(endpoint("http://127.0.0.1:11434/api/chat").is_ok());
        assert!(endpoint("https://api.example.com/v1/chat/completions").is_ok());
        assert!(parse(&vec![b' '; 256 * 1024 + 1]).is_err());
    }
    #[test]
    fn template_substitution_is_structural() {
        let value = serde_json::json!({"prompt":"$input","model":"$model","other":"prefix $input"});
        let result = expand(&value, "\"},\"attack\":true", "test").unwrap();
        assert_eq!(result["prompt"], "\"},\"attack\":true");
        assert_eq!(result["other"], "prefix $input");
        assert!(expand(&Value::String("$json".into()), "bad", "").is_err());
        assert!(
            expand(
                &serde_json::json!(["$input", "$input"]),
                &"a".repeat(200000),
                ""
            )
            .is_err()
        );
        let tool = parse(DEMO).unwrap().tools.remove(0);
        assert_eq!(execute(&tool, "z\na\nz", "", "").unwrap(), "z\na");
    }
    #[test]
    fn http_adapter_works_with_local_fixture() {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let read = socket.read(&mut request).unwrap();
            assert!(read > 0);
            let body = r#"{"message":{"content":"fixture answer"}}"#;
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        });
        let mut tool = parse(include_bytes!("../plugins-examples/ollama.json"))
            .unwrap()
            .tools
            .remove(1);
        if let Adapter::Http { url, .. } = &mut tool.adapter {
            *url = format!("http://{address}/api/chat");
        }
        assert_eq!(
            execute(&tool, "hello", "fixture", "").unwrap(),
            "fixture answer"
        );
        server.join().unwrap();
    }
}
