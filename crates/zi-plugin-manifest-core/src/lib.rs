//! Versioned ZiDevTools declarative manifests, independent of installation and execution.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

mod preview;
mod tool_kind;
pub use preview::{AdapterPreview, InspectError, Preview, ToolPreview, preview};
pub use tool_kind::ToolKind;

pub const CATEGORIES: &[&str] = &[
    "数据与格式",
    "文本与编码",
    "网络与接口",
    "文件与系统",
    "时间与生成",
    "AI 与模型",
    "知识与检索",
    "扩展与集成",
    "MCP 与 Agent",
    "Java 与 JVM",
    "Python 与 Django",
];
pub const MANIFEST_LIMIT: usize = 256 * 1024;
pub const PLUGIN_MANIFEST_SCHEMA_VERSION: u32 = 1;

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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginToolVersionRecord {
    pub version: String,
    pub date: String,
    pub changes: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginTool {
    pub id: String,
    /// Independent tool revision; old manifests remain readable without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Append-only, publisher-declared changes for this tool's independent version.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub version_history: Vec<PluginToolVersionRecord>,
    pub name: String,
    pub description: String,
    pub category: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Explicit localized or alternate names used for exact tool discovery.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
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
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

pub fn endpoint(url: &str) -> Result<url::Url> {
    let u = url::Url::parse(url).context("接口 URL 无效")?;
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

fn validate_plugin_tool_history(tool: &PluginTool) -> Result<()> {
    ensure!(
        tool.version_history.len() <= 64,
        "每个插件工具最多保留 64 条版本记录"
    );
    if tool.version_history.is_empty() {
        return Ok(());
    }
    let current = tool
        .version
        .as_deref()
        .context("插件工具声明版本历史时必须声明当前工具版本")?;
    let mut previous_version: Option<&str> = None;
    let mut previous_date: Option<chrono::NaiveDate> = None;
    for record in &tool.version_history {
        ensure!(
            valid_tool_version(&record.version),
            "插件工具历史版本需为 major.minor.patch"
        );
        let date = chrono::NaiveDate::parse_from_str(&record.date, "%Y-%m-%d")
            .context("插件工具版本历史日期须为 YYYY-MM-DD")?;
        ensure!(
            date.format("%Y-%m-%d").to_string() == record.date,
            "插件工具版本历史日期格式无效"
        );
        ensure!(
            !record.changes.trim().is_empty()
                && record.changes.len() <= 512
                && !record.changes.chars().any(char::is_control),
            "插件工具版本变更说明须为 1–512 字节且不含控制字符"
        );
        if let Some(previous) = previous_version {
            ensure!(
                compare_versions(&record.version, previous)? == std::cmp::Ordering::Greater,
                "插件工具版本历史必须严格递增"
            );
        }
        ensure!(
            previous_date.is_none_or(|previous| date >= previous),
            "插件工具版本历史日期必须按时间递增"
        );
        previous_version = Some(&record.version);
        previous_date = Some(date);
    }
    ensure!(
        tool.version_history
            .last()
            .map(|record| record.version.as_str())
            == Some(current),
        "插件工具当前版本必须与最后一条历史记录一致"
    );
    Ok(())
}

pub fn parse(bytes: &[u8]) -> Result<Manifest> {
    ensure!(bytes.len() <= MANIFEST_LIMIT, "插件清单最多 256 KiB");
    let m: Manifest = serde_json::from_slice(bytes).context("插件 JSON 格式或字段无效")?;
    validate_manifest(&m)?;
    Ok(m)
}

fn validate_manifest(m: &Manifest) -> Result<()> {
    ensure!(
        m.schema_version == PLUGIN_MANIFEST_SCHEMA_VERSION,
        "不支持的插件协议版本"
    );
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
            t.version.as_deref().is_none_or(valid_tool_version),
            "工具版本需为 major.minor.patch，不能有前导零"
        );
        validate_plugin_tool_history(t)?;
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
        ensure!(
            t.aliases.len() <= 16
                && t.aliases.iter().all(|alias| {
                    !alias.trim().is_empty()
                        && alias.len() <= 128
                        && !alias.chars().any(char::is_control)
                }),
            "工具别名最多 16 项，每项需为 1–128 字节的非控制文本"
        );
        match &t.adapter {
            Adapter::Builtin {
                tool,
                action,
                pattern,
            } => {
                let kind = ToolKind::ALL
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
    Ok(())
}

pub fn valid_tool_version(version: &str) -> bool {
    let parts: Vec<_> = version.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && (part.len() == 1 || !part.starts_with('0'))
                && part.bytes().all(|c| c.is_ascii_digit())
                && part.parse::<u32>().is_ok()
        })
}

pub fn compare_versions(left: &str, right: &str) -> Result<std::cmp::Ordering> {
    fn parts(value: &str) -> Result<[u32; 3]> {
        ensure!(valid_tool_version(value), "插件版本需为 major.minor.patch");
        let mut values = value.split('.').map(str::parse::<u32>);
        Ok([
            values.next().context("插件版本无效")??,
            values.next().context("插件版本无效")??,
            values.next().context("插件版本无效")??,
        ])
    }
    Ok(parts(left)?.cmp(&parts(right)?))
}
