//! Inspects a single confirmed native text-flow recipe.

use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Read;

use zi_flow_recipe_core::{RECIPE_LIMIT, preview};

const PLUGIN: &str = "com.zicode.devtools.flow-recipe";
const CAPABILITY: &str = "devtools.flow.recipe.inspect";
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
    if !matches!(file.mime_type.as_str(), "application/json" | "text/plain") {
        return Err("UNSUPPORTED_MIME");
    }
    if file.byte_length > RECIPE_LIMIT {
        return Err("INPUT_TOO_LARGE");
    }
    let file = read(file.byte_length)?;
    let preview = preview(&file).map_err(|_| "INVALID_RECIPE")?;
    let mut result = serde_json::to_value(preview).map_err(|_| "RESULT_FAILED")?;
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
            "INPUT_TOO_LARGE" => "The recipe exceeds 64 KiB. 配方超过 64 KiB。",
            "INVALID_RECIPE" => "Use a valid text-flow recipe of version 1 to 4 with compatible steps. 请选择第 1 至 4 版且操作相容的文本流程配方。",
            "UNSUPPORTED_MIME" => "Choose a local recipe JSON file. 请选择本机配方 JSON 文件。",
            _ => "The recipe could not be inspected. 无法检查配方。",
        };
        json!({"ok":false,"error":{"code":code,"message":message}})
    });
    serde_json::to_writer(std::io::stdout().lock(), &output).expect("write result");
}
