//! Previews an explicitly approved local memo text file.
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{Read, Write};
use zi_memo_text_core::{ImportError, MAX_FILE_BYTES, prepare};

const PLUGIN: &str = "com.zicode.devtools.memo-text";
const CAPABILITY: &str = "devtools.memo.file.preview";
const PROFILE: &str = "resource.wasi.input.draft2";
const CONTRACT: &str = "1.0.0-rc.1";
const MAX_PREVIEW: usize = 6 * 1024;
const MAX_RESULT: usize = 48 * 1024;
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
struct Input {
    #[serde(default)]
    title: String,
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
    const REQUEST: Failure = ("INVALID_REQUEST", "备忘导入预览请求无效。");
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
    let Input { title } = request.input;
    if title.chars().count() > 120 {
        return Err(REQUEST);
    }
    const RESOURCE: Failure = ("INVALID_RESOURCE", "请选择并确认一份本机文本文件。");
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
        "text/plain" | "text/markdown" | "application/octet-stream"
    ) {
        return Err(("UNSUPPORTED_MIME", "请选择 UTF-8 的 Markdown 或文本文件。"));
    }
    if file.byte_length > MAX_FILE_BYTES {
        return Err(("INPUT_TOO_LARGE", ImportError::TooLarge.message()));
    }
    let bytes = read(file.byte_length)
        .map_err(|_| ("RESOURCE_READ_FAILED", "无法读取本次确认的文本文件。"))?;
    let memo = prepare(&bytes, &title).map_err(|error| {
        let code = match error {
            ImportError::TooLarge => "INPUT_TOO_LARGE",
            ImportError::InvalidUtf8 => "INVALID_UTF8",
            ImportError::ContainsNul => "CONTAINS_NUL",
        };
        (code, error.message())
    })?;
    let body = memo.body;
    let mut end = body.len().min(MAX_PREVIEW);
    while !body.is_char_boundary(end) {
        end -= 1;
    }
    let truncated = end < body.len();
    let shown = &body[..end];
    let mut text = format!("{}\n正文显示 {end}/{} 字节\n\n", memo.title, body.len());
    if shown.is_empty() {
        text.push_str("（空白正文）");
    } else {
        text.push_str(shown);
    }
    if truncated {
        text.push_str("\n\n[已省略剩余正文]");
    }
    Ok(
        json!({"title":memo.title,"byteLength":bytes.len(),"bodyByteLength":body.len(),
              "previewByteLength":end,"hadBom":memo.had_bom,"truncated":truncated,"text":text}),
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

fn main() {
    let result = match run() {
        Ok(data) => json!({"contractVersion":CONTRACT,"ok":true,"data":data,"error":null}),
        Err(reason) => error(reason),
    };
    let mut bytes = serde_json::to_vec(&result).expect("valid memo text preview");
    if bytes.len() > MAX_RESULT {
        bytes = serde_json::to_vec(&error(("RESULT_TOO_LARGE", "预览结果超过输出限制。")))
            .expect("fixed result error");
    }
    if std::io::stdout().write_all(&bytes).is_err() {
        std::process::exit(1);
    }
}
