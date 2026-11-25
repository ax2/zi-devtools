//! Selects a saved memo from a confirmed portable backup and emits its exact body.
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{Read, Write};
use zi_memo_text_core::backup::{ExtractProblem, MAX_EXTRACT_BYTES, read_snapshot};

const PLUGIN: &str = "com.zicode.devtools.memo-backup";
const CAPABILITY: &str = "devtools.memos.backup.markdown";
const PROFILE: &str = "resource.wasi.input.draft2";
const CONTRACT: &str = "1.0.0-rc.1";
const OUTPUT_PROFILE: &str = "resource.wasi.output.draft1";
const MAX_RESULT: usize = 40 * 1024;
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
    memo_number: Option<usize>,
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
    const REQUEST: Failure = ("INVALID_REQUEST", "备忘备份提取请求无效。");
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
        || request.resource_output_profile.as_deref() != Some(OUTPUT_PROFILE)
    {
        return Err(REQUEST);
    }
    const RESOURCE: Failure = ("INVALID_RESOURCE", "请选择并确认一份本机备忘备份。");
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
        "application/octet-stream" | "application/json" | "text/plain"
    ) {
        return Err((
            "UNSUPPORTED_MIME",
            "请选择 ZiDevTools 的备忘与日程 JSON 备份。",
        ));
    }
    if file.byte_length > MAX_EXTRACT_BYTES {
        return Err(("INPUT_TOO_LARGE", "单份备份最多 128 KiB。"));
    }
    let bytes = read(file.byte_length)
        .map_err(|_| ("RESOURCE_READ_FAILED", "无法读取已确认的备忘备份。"))?;
    let snapshot = read_snapshot(&bytes).map_err(problem)?;
    let mut eligible = snapshot
        .records
        .iter()
        .enumerate()
        .filter(|(_, record)| record.can_export());
    let eligible_count = eligible.clone().count();
    if let Some(number) = request.input.memo_number {
        let row = number
            .checked_sub(1)
            .and_then(|index| snapshot.records.get(index))
            .ok_or(("MEMO_NOT_FOUND", "备忘序号超出备份范围。"))?;
        if !row.can_export() {
            return Err((
                "MEMO_NOT_EXPORTABLE",
                "该序号属于日程或回收站。请选择可导出的普通备忘。",
            ));
        }
        let body = row.body.as_bytes();
        publish_markdown(body)?;
        let text = format!(
            "已提取第 {number} 条备忘 · {} · {} 字节。请预览后另存 Markdown。",
            row.title,
            body.len()
        );
        return Ok(
            json!({"memoNumber":number,"totalRecords":snapshot.records.len(),"eligibleMemos":eligible_count,
            "byteLength":body.len(),"text":text}),
        );
    }
    use std::fmt::Write as _;
    let mut text = format!(
        "备份共 {} 条记录 · 可提取 {eligible_count} 条普通备忘",
        snapshot.records.len()
    );
    if eligible_count > 0 {
        text.push_str("\n输入下列对应序号并再次执行，即可提取完整 Markdown。");
        for (index, row) in &mut eligible {
            let _ = write!(text, "\n{} · {}", index + 1, row.title);
        }
    }
    Ok(
        json!({"memoNumber":null,"totalRecords":snapshot.records.len(),"eligibleMemos":eligible_count,
        "byteLength":null,"text":text}),
    )
}

fn problem(error: ExtractProblem) -> Failure {
    match error {
        ExtractProblem::TooLarge => ("INPUT_TOO_LARGE", "单份备份最多 128 KiB。"),
        ExtractProblem::TooManyRecords => ("RECORD_LIMIT", "每份备份最多检查 64 条记录。"),
        ExtractProblem::InvalidBackup => (
            "INVALID_BACKUP",
            "备份结构或记录字段不符合 ZiDevTools 格式。",
        ),
        ExtractProblem::UnsupportedFormat => (
            "UNSUPPORTED_VERSION",
            "请选择 ZiDevTools 版本 1 的备忘与日程备份。",
        ),
    }
}

const OUTPUT_FAILED: Failure = ("OUTPUT_FAILED", "无法生成 Markdown，请再次确认文件后重试。");

#[cfg(target_arch = "wasm32")]
fn publish_markdown(bytes: &[u8]) -> Result<(), Failure> {
    // The host's append-only resource.output.types.v1 index for text/markdown.
    const MARKDOWN: i32 = 24;
    #[link(wasm_import_module = "zicode_resource_output_draft1")]
    unsafe extern "C" {
        #[link_name = "create"]
        fn output_create(mime: i32, length: i64) -> i32;
        #[link_name = "write"]
        fn output_write(index: i32, source: *const u8, length: usize) -> i32;
        #[link_name = "finish"]
        fn output_finish(index: i32) -> i32;
    }
    let index = unsafe { output_create(MARKDOWN, bytes.len() as i64) };
    if index < 0 {
        return Err(OUTPUT_FAILED);
    }
    for chunk in bytes.chunks(64 * 1024) {
        if unsafe { output_write(index, chunk.as_ptr(), chunk.len()) } != chunk.len() as i32 {
            return Err(OUTPUT_FAILED);
        }
    }
    if unsafe { output_finish(index) } != 0 {
        return Err(OUTPUT_FAILED);
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn publish_markdown(bytes: &[u8]) -> Result<(), Failure> {
    let target = std::env::args_os().nth(2).ok_or(OUTPUT_FAILED)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .map_err(|_| OUTPUT_FAILED)?;
    file.write_all(bytes).map_err(|_| OUTPUT_FAILED)
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
    let mut bytes = serde_json::to_vec(&result).expect("valid memo extraction report");
    if bytes.len() > MAX_RESULT {
        bytes = serde_json::to_vec(&error(("RESULT_TOO_LARGE", "备忘提取结果超过上限。")))
            .expect("fixed result error");
    }
    if std::io::stdout().write_all(&bytes).is_err() {
        std::process::exit(1);
    }
}
