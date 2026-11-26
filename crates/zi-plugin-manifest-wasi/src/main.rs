//! Read one approved ZiDevTools manifest and present its declaration without execution.
use serde::Deserialize;
use serde_json::{Value, json};
use std::fmt::Write as _;
use std::io::{Read, Write};
use zi_plugin_manifest_core::{AdapterPreview, InspectError, MANIFEST_LIMIT, preview};

const PLUGIN: &str = "com.zicode.devtools.plugin-manifest-preview";
const CAPABILITY: &str = "devtools.plugin-manifest.preview";
const PROFILE: &str = "resource.wasi.input.draft2";
const CONTRACT: &str = "1.0.0-rc.1";
const MAX_RESULT: usize = 8192;
type Failure = (&'static str, &'static str);

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Request {
    plugin_id: String,
    scene_id: String,
    capability_id: String,
    command_id: Value,
    input: Input,
    resource_input_profile: String,
    resource_output_profile: Option<String>,
    resources: Vec<Descriptor>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Input {
    #[serde(default)]
    tool_start: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Descriptor {
    index: usize,
    resource_id: String,
    content_version: String,
    mime_type: String,
    byte_length: usize,
}

fn run() -> Result<Value, Failure> {
    const MAX_REQUEST: usize = 16 * 1024;
    const REQUEST: Failure = ("INVALID_REQUEST", "清单预览请求无效。");
    const RESOURCE: Failure = ("INVALID_RESOURCE", "请选择并确认一份 JSON 清单。");
    let mut envelope = Vec::new();
    std::io::stdin()
        .take((MAX_REQUEST + 1) as u64)
        .read_to_end(&mut envelope)
        .map_err(|_| REQUEST)?;
    if envelope.len() > MAX_REQUEST {
        return Err(REQUEST);
    }
    let request: Request = serde_json::from_slice(&envelope).map_err(|_| REQUEST)?;
    if request.plugin_id != PLUGIN
        || request.capability_id != CAPABILITY
        || request.scene_id.is_empty()
        || request.scene_id.len() > 128
        || !request.command_id.is_null()
        || request.resource_input_profile != PROFILE
        || request.resource_output_profile.is_some()
    {
        return Err(REQUEST);
    }
    let [file] = request.resources.as_slice() else {
        return Err(RESOURCE);
    };
    if file.index != 0
        || file.resource_id.is_empty()
        || file.content_version.len() != 64
        || !file
            .content_version
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(RESOURCE);
    }
    if !matches!(
        file.mime_type.as_str(),
        "application/json" | "application/octet-stream"
    ) {
        return Err(("UNSUPPORTED_MIME", "请选择 ZiDevTools 声明式 JSON 清单。"));
    }
    if file.byte_length > MANIFEST_LIMIT {
        return Err(("INPUT_TOO_LARGE", "单份清单最多 256 KiB。"));
    }
    let bytes =
        read(file.byte_length).map_err(|_| ("RESOURCE_READ_FAILED", "无法读取已确认的清单。"))?;
    let report =
        preview(&bytes, request.input.tool_start.unwrap_or(1)).map_err(|error| match error {
            InspectError::TooLarge => ("INPUT_TOO_LARGE", "单份清单最多 256 KiB。"),
            InspectError::TooComplex => (
                "DOCUMENT_TOO_COMPLEX",
                "清单中的结构、转义或引号外字符过多，请缩小复杂声明后重试。",
            ),
            InspectError::InvalidJson => ("INVALID_JSON", "清单必须是不含重复键的有效 JSON。"),
            InspectError::InvalidManifest => {
                ("INVALID_MANIFEST", "清单不符合 ZiDevTools 声明式插件格式。")
            }
            InspectError::InvalidPage => ("INVALID_TOOL_START", "工具起始序号超出此清单范围。"),
        })?;
    let mut text = format!(
        "{} · {} · v{}\nSHA-256 {}\n{} 个声明：内置 {}，HTTP {}。",
        report.name,
        report.id,
        report.version,
        file.content_version,
        report.total_tools,
        report.builtin_tools,
        report.http_tools
    );
    for tool in &report.tools {
        let _ = write!(
            text,
            "\n{}. {} · {} · {}",
            tool.ordinal, tool.id, tool.name, tool.category
        );
        match &tool.adapter {
            AdapterPreview::Builtin { tool, action } => {
                let _ = write!(text, " · 内置 {tool}#{action}");
            }
            AdapterPreview::Http { method } => {
                let _ = write!(text, " · HTTP {method}");
            }
        }
    }
    if let Some(next) = report.next_tool_start {
        let _ = write!(text, "\n继续查看时，输入工具起始序号 {next}。");
    }
    Ok(
        json!({"preview": report, "byteLength": bytes.len(), "sha256": file.content_version, "text": text}),
    )
}

#[cfg(target_arch = "wasm32")]
fn read(length: usize) -> Result<Vec<u8>, &'static str> {
    zi_resource_input_wasi::read_window(0, length, 0, length)
}

#[cfg(not(target_arch = "wasm32"))]
fn read(length: usize) -> Result<Vec<u8>, &'static str> {
    const FAILED: &str = "RESOURCE_READ_FAILED";
    let path = std::env::args_os().nth(1).ok_or(FAILED)?;
    let file = std::fs::File::open(path).map_err(|_| FAILED)?;
    let mut bytes = Vec::new();
    file.take((length + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| FAILED)?;
    if bytes.len() != length {
        return Err(FAILED);
    }
    Ok(bytes)
}

fn error((code, message): Failure) -> Value {
    json!({"contractVersion":CONTRACT,"ok":false,"data":null,"error":{"code":code,"message":message}})
}

fn main() -> std::io::Result<()> {
    let result = match run() {
        Ok(data) => json!({"contractVersion":CONTRACT,"ok":true,"data":data,"error":null}),
        Err(failure) => error(failure),
    };
    let mut bytes = serde_json::to_vec(&result)?;
    if bytes.len() > MAX_RESULT {
        bytes = serde_json::to_vec(&error(("RESULT_TOO_LARGE", "清单摘要超过 8 KiB。")))?;
    }
    std::io::stdout().write_all(&bytes)
}
