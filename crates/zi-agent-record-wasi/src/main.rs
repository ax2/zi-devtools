//! Summarizes an approved Agent export without echoing any free-text fields.
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fmt::Write as _,
    io::{Read, Write},
};
use zi_agent_record_core::{ImportedRecord, parse_json};

const PLUGIN: &str = "com.zicode.devtools.agent-record";
const CAPABILITY: &str = "devtools.agent-record.inspect";
const PROFILE: &str = "resource.wasi.input.draft2";
const CONTRACT: &str = "1.0.0-rc.1";
const MAX_FILE: usize = 64 * 1024;
const MAX_RESULT: usize = 4096;
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

fn summarize(record: ImportedRecord, bytes: usize, finished_at: String) -> Value {
    let steps:Vec<_> = record.steps.iter().enumerate().map(|(index, step)| {
        let tool=record.allowed_tools.iter().position(|name| name==&step.tool).expect("validated allowlist") + 1;
        json!({"step":index+1,"toolNumber":tool,"elapsedMs":step.elapsed_ms as u64,"contentItems":step.content_items,
               "responseBytes":step.response_bytes,"modelExcerptBytes":step.model_excerpt_bytes,"isError":step.is_error})
    }).collect();
    let refs: usize = record
        .steps
        .iter()
        .map(|step| step.references.as_ref().map_or(0, Vec::len))
        .sum();
    let citations = record
        .content
        .as_ref()
        .and_then(|content| content.citations.as_ref())
        .map_or(0, Vec::len);
    let tokens = record
        .model_tokens_reported
        .map(|tokens| tokens.to_string());
    let version = record
        .generated_by
        .as_ref()
        .map(|provenance| provenance.tool_version.as_str());
    let status = match record.status.as_str() {
        "completed" => "已完成",
        "failed" => "失败",
        _ => "已取消",
    };
    let mut text = format!(
        "Agent 记录 v{} · {status}\n结束时间（UTC） {}\n记录标记已人工批准\n调用 {}/{} · 白名单 {} 项\n模型报告用量 {}\n来源声明 {} 条 · 引用编号 {} 条",
        record.schema_version,
        finished_at,
        record.calls_made,
        record.max_calls,
        record.allowed_tools.len(),
        tokens.as_deref().unwrap_or("未知"),
        refs,
        citations
    );
    if let Some(version) = version {
        let _ = write!(text, "\n生成工具版本 {version} · 环境指纹结构校验通过");
    }
    for step in &steps {
        let _ = write!(
            text,
            "\n{}. 工具 {} · {} ms · 响应 {} 字节{}",
            step["step"],
            step["toolNumber"],
            step["elapsedMs"],
            step["responseBytes"],
            if step["isError"] == true {
                " · 工具报告错误"
            } else {
                ""
            }
        );
    }
    text.push_str("\n工具按原白名单序号显示。记录仅通过结构核验。");
    json!({"recordSchemaVersion":record.schema_version,"byteLength":bytes,"finishedAtUtc":finished_at,
           "status":record.status,"approvalRecorded":record.approved,"allowedToolCount":record.allowed_tools.len(),
           "maxCalls":record.max_calls,"callsMade":record.calls_made,"modelTokensReported":tokens,"steps":steps,
           "sourceDeclarationCount":refs,"numberedCitationCount":citations,"generatorToolVersion":version,
           "fingerprintStructurallyVerified":record.generated_by.is_some(),"text":text})
}

fn run() -> Result<Value, Failure> {
    const MAX_REQUEST: usize = 16 * 1024;
    const REQUEST: Failure = ("INVALID_REQUEST", "文件检查请求无效。");
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
    let Input {} = request.input;
    const RESOURCE: Failure = ("INVALID_RESOURCE", "请选择并确认一份运行记录。");
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
        return Err(("UNSUPPORTED_MIME", "请选择导出的 JSON 记录。"));
    }
    if file.byte_length > MAX_FILE {
        return Err(("INPUT_TOO_LARGE", "单份记录最多 64 KiB。"));
    }
    let bytes =
        read(file.byte_length).map_err(|_| ("RESOURCE_READ_FAILED", "无法读取已确认的记录。"))?;
    let record = parse_json(&bytes)
        .map_err(|_| ("INVALID_RECORD", "文件不符合可支持的 Agent 记录结构。"))?;
    let finished_at = record
        .normalized_finished_at_utc()
        .map_err(|_| ("INVALID_RECORD", "文件不符合可支持的 Agent 记录结构。"))?;
    Ok(summarize(record, bytes.len(), finished_at))
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
    let mut bytes = serde_json::to_vec(&result).expect("valid Agent record summary");
    if bytes.len() > MAX_RESULT {
        bytes = serde_json::to_vec(&error(("RESULT_TOO_LARGE", "摘要超过 4 KiB。")))
            .expect("fixed result error");
    }
    if std::io::stdout().write_all(&bytes).is_err() {
        std::process::exit(1);
    }
}
