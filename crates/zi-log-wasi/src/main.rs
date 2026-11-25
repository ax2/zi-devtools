//! Report on an approved text snapshot. The guest has no file path or write authority.

use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Read;
use zi_log_core::report::{self, Filter, MAX_FILE_BYTES, tail};

const PLUGIN: &str = "com.zicode.devtools.log-viewer";
const FILTER: &str = "devtools.log-viewer.filter";
const TAIL: &str = "devtools.log-viewer.tail";
const PROFILE: &str = "resource.wasi.input.draft2";
type Failure = &'static str;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    plugin_id: String,
    scene_id: String,
    capability_id: String,
    command_id: Value,
    input: Filter,
    resource_input_profile: String,
    resource_output_profile: Option<String>,
    resources: Vec<Descriptor>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Descriptor {
    index: usize,
    resource_id: String,
    content_version: String,
    mime_type: String,
    byte_length: usize,
}

fn execute() -> Result<Value, Failure> {
    const MAX_ENVELOPE: usize = 16 * 1024;
    let mut envelope = Vec::new();
    std::io::stdin()
        .take((MAX_ENVELOPE + 1) as u64)
        .read_to_end(&mut envelope)
        .map_err(|_| "INVALID_REQUEST")?;
    if envelope.len() > MAX_ENVELOPE {
        return Err("INVALID_REQUEST");
    }
    let request: Request = serde_json::from_slice(&envelope).map_err(|_| "INVALID_REQUEST")?;
    if request.plugin_id != PLUGIN
        || request.scene_id.is_empty()
        || !request.command_id.is_null()
        || request.resource_input_profile != PROFILE
        || request.resource_output_profile.is_some()
    {
        return Err("INVALID_REQUEST");
    }
    let [file] = request.resources.as_slice() else {
        return Err("INVALID_RESOURCE");
    };
    if file.index != 0
        || file.resource_id.is_empty()
        || file.content_version.len() != 64
        || !file
            .content_version
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("INVALID_RESOURCE");
    }
    if !matches!(
        file.mime_type.as_str(),
        "text/plain" | "application/octet-stream"
    ) {
        return Err("UNSUPPORTED_MIME");
    }
    let mut reply = match request.capability_id.as_str() {
        FILTER => {
            if file.byte_length > MAX_FILE_BYTES {
                return Err("INPUT_TOO_LARGE");
            }
            let bytes = read(file.byte_length, 0)?;
            serde_json::to_value(report::scan(&bytes, &request.input)?)
        }
        TAIL => {
            let start = tail::read_start(file.byte_length)?;
            let bytes = read(file.byte_length, start)?;
            serde_json::to_value(tail::scan_tail(&bytes, file.byte_length, &request.input)?)
        }
        _ => return Err("INVALID_REQUEST"),
    }
    .map_err(|_| "RESULT_FAILED")?;
    reply["ok"] = json!(true);
    Ok(reply)
}

#[cfg(target_arch = "wasm32")]
fn read(length: usize, start: usize) -> Result<Vec<u8>, Failure> {
    zi_resource_input_wasi::read_window(0, length, start, length - start)
}

// Host-independent adapter for comparing the same report against the production Worker.
#[cfg(not(target_arch = "wasm32"))]
fn read(length: usize, start: usize) -> Result<Vec<u8>, Failure> {
    use std::io::{Seek, SeekFrom};

    let path = std::env::args_os().nth(1).ok_or("RESOURCE_READ_FAILED")?;
    let mut file = std::fs::File::open(path).map_err(|_| "RESOURCE_READ_FAILED")?;
    if file.metadata().map_err(|_| "RESOURCE_READ_FAILED")?.len() != length as u64 {
        return Err("RESOURCE_READ_FAILED");
    }
    file.seek(SeekFrom::Start(start as u64))
        .map_err(|_| "RESOURCE_READ_FAILED")?;
    let mut bytes = Vec::new();
    file.take((length - start + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "RESOURCE_READ_FAILED")?;
    if bytes.len() != length - start {
        return Err("RESOURCE_READ_FAILED");
    }
    Ok(bytes)
}

fn main() {
    let reply = execute().unwrap_or_else(|code| {
        let message = match code {
            "INPUT_TOO_LARGE" => "Log file exceeds 16 KiB. 日志超过 16 KiB。",
            "SNAPSHOT_TOO_LARGE" => "Log snapshot exceeds 32 MiB. 日志快照超过 32 MiB。",
            "QUERY_TOO_LARGE" => {
                "Search phrase exceeds 256 UTF-8 bytes. 搜索词超过 256 个 UTF-8 字节。"
            }
            "INVALID_UTF8" => "Log file is not valid UTF-8. 日志不是有效的 UTF-8 文件。",
            "UNSUPPORTED_MIME" => "Select a plain-text log file. 请选择纯文本日志。",
            _ => "Log preview could not be produced. 无法预览日志。",
        };
        json!({"ok":false,"error":{"code":code,"message":message}})
    });
    serde_json::to_writer(std::io::stdout().lock(), &reply).expect("write result");
}
