//! Reads only the confirmed Word document body and returns bounded chunks.

use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Read;

mod preview;
use preview::MAX_DOCX_BYTES;

const PLUGIN: &str = "com.zicode.devtools.docx-preview";
const CAPABILITY: &str = "devtools.docx.preview.chunks";
type Error = &'static str;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Envelope {
    plugin_id: String,
    scene_id: String,
    capability_id: String,
    command_id: Value,
    #[serde(rename = "input")]
    _input: Input,
    resource_input_profile: String,
    resource_output_profile: Option<String>,
    resources: Vec<Resource>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Resource {
    index: usize,
    resource_id: String,
    content_version: String,
    mime_type: String,
    byte_length: usize,
}

fn execute() -> Result<Value, Error> {
    const ENVELOPE_BYTES: usize = 16 * 1024;
    let mut encoded = Vec::new();
    std::io::stdin()
        .take((ENVELOPE_BYTES + 1) as u64)
        .read_to_end(&mut encoded)
        .map_err(|_| "INVALID_REQUEST")?;
    if encoded.len() > ENVELOPE_BYTES {
        return Err("INVALID_REQUEST");
    }
    let request: Envelope = serde_json::from_slice(&encoded).map_err(|_| "INVALID_REQUEST")?;
    if request.plugin_id != PLUGIN
        || request.scene_id.is_empty()
        || request.capability_id != CAPABILITY
        || !request.command_id.is_null()
        || request.resource_input_profile != "resource.wasi.input.draft2"
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
    // Studio's generic file picker labels an unknown binary type as octet-stream.
    if file.mime_type != "application/octet-stream" {
        return Err("UNSUPPORTED_MIME");
    }
    if file.byte_length > MAX_DOCX_BYTES {
        return Err("INPUT_TOO_LARGE");
    }
    let file = read(file.byte_length)?;
    let mut result = serde_json::to_value(preview::run(&file)?).map_err(|_| "RESULT_FAILED")?;
    result["ok"] = json!(true);
    Ok(result)
}

#[cfg(target_arch = "wasm32")]
fn read(length: usize) -> Result<Vec<u8>, Error> {
    zi_resource_input_wasi::read_prefix(0, length, length)
}

#[cfg(not(target_arch = "wasm32"))]
fn read(length: usize) -> Result<Vec<u8>, Error> {
    // Isolated native counterpart for comparing the same vectors against the real Worker.
    let path = std::env::args_os().nth(1).ok_or("RESOURCE_READ_FAILED")?;
    let file = std::fs::File::open(path).map_err(|_| "RESOURCE_READ_FAILED")?;
    let mut bytes = Vec::new();
    file.take((length + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "RESOURCE_READ_FAILED")?;
    if bytes.len() != length {
        return Err("RESOURCE_READ_FAILED");
    }
    Ok(bytes)
}

fn main() {
    let output = execute().unwrap_or_else(|code| {
        let message = match code {
            "INPUT_TOO_LARGE" => "DOCX exceeds 128 KiB. DOCX 超过 128 KiB。",
            "XML_TOO_LARGE" => {
                "Expanded document XML exceeds 128 KiB. 解压后的正文 XML 超过 128 KiB。"
            }
            "ARCHIVE_ENTRIES_EXCEEDED" => {
                "The package has more than 256 entries. 文件包条目超过 256 项。"
            }
            "INVALID_ARCHIVE" => "The Word ZIP package could not be read. 无法读取 Word 文件包。",
            "MISSING_DOCUMENT_BODY" => "Missing word/document.xml. 文件缺少 Word 正文。",
            "INVALID_DOCUMENT_XML" => "The document body XML is invalid. Word 正文 XML 格式无效。",
            "PARAGRAPH_LIMIT" | "CHUNK_LIMIT" => {
                "The document has too many paragraphs or chunks. 正文段落或分块过多。"
            }
            "UNSUPPORTED_MIME" => "Choose a local DOCX file. 请选择本机 DOCX 文件。",
            _ => "DOCX preview could not be produced. 无法预览 DOCX。",
        };
        json!({"ok":false,"error":{"code":code,"message":message}})
    });
    serde_json::to_writer(std::io::stdout().lock(), &output).expect("write result");
}
