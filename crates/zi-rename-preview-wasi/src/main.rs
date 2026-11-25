//! Previews name changes from a single confirmed folder snapshot.

use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Read;

mod preview;

use preview::{MAX_INPUT, MAX_RESULT, Options};

const PLUGIN: &str = "com.zicode.devtools.rename-preview";
const CAPABILITY: &str = "devtools.files.rename.preview";
type Error = &'static str;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Envelope {
    plugin_id: String,
    scene_id: String,
    capability_id: String,
    command_id: Value,
    input: Options,
    resource_input_profile: String,
    resource_output_profile: Option<String>,
    resources: Vec<Resource>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Resource {
    index: usize,
    resource_id: String,
    content_version: String,
    mime_type: String,
    byte_length: usize,
}

fn execute() -> Result<Vec<u8>, Error> {
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
    if file.mime_type != "text/plain" {
        return Err("UNSUPPORTED_MIME");
    }
    if file.byte_length > MAX_INPUT {
        return Err("INPUT_TOO_LARGE");
    }
    let file = read(file.byte_length)?;
    let report = preview::analyze(&file, &request.input)?;
    let output = serde_json::to_vec(&report).map_err(|_| "RESULT_FAILED")?;
    if output.len() > MAX_RESULT {
        return Err("RESULT_TOO_LARGE");
    }
    Ok(output)
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
            "INPUT_TOO_LARGE" => "The folder snapshot exceeds 64 KiB. 目录快照超过 64 KiB。",
            "INVALID_SNAPSHOT" | "UNSUPPORTED_MIME" => "Choose a folder and confirm sharing its names. 请从「文件夹」选择并确认共享名称。",
            "TOO_MANY_ENTRIES" => "This preview accepts up to 64 folder entries. 本次预览最多支持 64 个目录项。",
            "NAME_BUDGET_EXCEEDED" => "Folder entry names total more than 12 KiB. 目录项名称合计超过 12 KiB。",
            "TARGET_BUDGET_EXCEEDED" => "The proposed names total more than 16 KiB. 候选文件名称合计超过 16 KiB。",
            "RULE_TOO_LARGE" => "Each rule field must be at most 128 UTF-8 bytes. 每项规则最多 128 个 UTF-8 字节。",
            "RESULT_TOO_LARGE" => "The report exceeds 48 KiB. Use a folder with fewer or shorter names. 结果超过 48 KiB，请使用目录项更少或名称更短的文件夹。",
            _ => "Rename preview could not be produced. 无法生成改名预览。",
        };
        serde_json::to_vec(&json!({"ok":false,"error":{"code":code,"message":message}}))
            .expect("serialize error")
    });
    std::io::Write::write_all(&mut std::io::stdout().lock(), &output).expect("write report");
}
