//! Reviews a confirmed export without querying an index or a model.
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{Read, Write};
use zi_rag_eval_core::{MAX_REPORT_BYTES, review};

const PLUGIN: &str = "com.zicode.devtools.retrieval-review";
const CAPABILITY: &str = "devtools.rag.report.review";
const PROFILE: &str = "resource.wasi.input.draft2";
const CONTRACT: &str = "1.0.0-rc.1";
const MAX_RESULT: usize = 8 * 1024;
type Failure = (&'static str, &'static str);

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Request {
    plugin_id: String,
    scene_id: String,
    capability_id: String,
    command_id: Value,
    #[serde(rename = "input")]
    _input: Input,
    resource_input_profile: String,
    resource_output_profile: Option<String>,
    resources: Vec<Descriptor>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {}

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
    const REQUEST: Failure = ("INVALID_REQUEST", "检索评测报告复核请求无效。");
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
    const RESOURCE: Failure = ("INVALID_RESOURCE", "请选择并确认一份本机检索评测报告。");
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
        "application/json" | "application/octet-stream" | "text/plain"
    ) {
        return Err((
            "UNSUPPORTED_MIME",
            "请选择 ZiDevTools 导出的检索评测 JSON 文件。",
        ));
    }
    if file.byte_length > MAX_REPORT_BYTES {
        return Err(("INPUT_TOO_LARGE", "单份检索评测报告最多 64 KiB。"));
    }
    let bytes = read(file.byte_length)
        .map_err(|_| ("RESOURCE_READ_FAILED", "无法读取已确认的检索评测报告。"))?;
    let report = review::read(&bytes).map_err(|code| (code, message(code)))?;
    serde_json::to_value(report).map_err(|_| ("INVALID_REPORT", "报告统计结果无效。"))
}

fn message(code: &str) -> &'static str {
    match code {
        "UNSUPPORTED_VERSION" => "支持检索报告版本 2 或双路对比报告版本 1。",
        "UNSUPPORTED_MODE" => "请选择仅检索报告或双路检索对比报告。",
        "INPUT_TOO_LARGE" => "单份检索评测报告最多 64 KiB。",
        _ => "报告结构无效。请检查题数、名次、证据顺序及检索设置。",
    }
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

fn main() {
    let result = match run() {
        Ok(data) => json!({"contractVersion":CONTRACT,"ok":true,"data":data,"error":null}),
        Err(reason) => error(reason),
    };
    let mut bytes = serde_json::to_vec(&result).expect("valid retrieval review");
    if bytes.len() > MAX_RESULT {
        bytes = serde_json::to_vec(&error(("RESULT_TOO_LARGE", "检索评测复核超过结果上限。")))
            .expect("fixed result error");
    }
    if std::io::stdout().write_all(&bytes).is_err() {
        std::process::exit(1);
    }
}
