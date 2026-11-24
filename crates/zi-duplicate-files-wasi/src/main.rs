use std::io::{Read, Write};
use zi_duplicate_files_wasi::{MAX_REPORT_BYTES, MAX_SNAPSHOT_BYTES, analyze};

fn read_stdin(limit: usize) -> Result<Vec<u8>, &'static str> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "INVALID_REQUEST")?;
    if bytes.len() > limit {
        return Err("INPUT_TOO_LARGE");
    }
    Ok(bytes)
}

fn main() {
    let result = transport::snapshot()
        .and_then(|bytes| analyze(&bytes))
        .and_then(|report| {
            let result = serde_json::to_vec(&report).map_err(|_| "OUTPUT_FAILED")?;
            if result.len() > MAX_REPORT_BYTES {
                return Err("RESULT_TOO_LARGE");
            }
            Ok(result)
        });
    let bytes = result.unwrap_or_else(|code| {
        let message = match code {
            "INVALID_DIRECTORY_SNAPSHOT" => {
                "The approved digest snapshot is invalid. 无法读取目录摘要快照。"
            }
            "INPUT_TOO_LARGE" => {
                "The approved digest snapshot exceeds 60 KiB. 目录摘要快照超过 60 KiB。"
            }
            _ => "Duplicate file analysis could not finish. 重复文件检查未完成。",
        };
        serde_json::to_vec(&serde_json::json!({"ok":false,"error":{"code":code,"message":message}}))
            .expect("serialize error")
    });
    std::io::stdout()
        .lock()
        .write_all(&bytes)
        .expect("write report");
}

#[cfg(target_arch = "wasm32")]
mod transport {
    use super::*;
    use serde::Deserialize;
    use serde_json::Value;
    use zi_duplicate_files_wasi::{CAPABILITY, PLUGIN};

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Envelope {
        plugin_id: String,
        scene_id: String,
        capability_id: String,
        command_id: Value,
        input: Value,
        resource_input_profile: String,
        resource_output_profile: Option<String>,
        resources: Vec<Descriptor>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Descriptor {
        index: i32,
        resource_id: String,
        content_version: String,
        mime_type: String,
        byte_length: usize,
    }

    pub(super) fn snapshot() -> Result<Vec<u8>, &'static str> {
        let envelope: Envelope =
            serde_json::from_slice(&read_stdin(16 * 1024)?).map_err(|_| "INVALID_REQUEST")?;
        let [resource] = envelope.resources.as_slice() else {
            return Err("INVALID_REQUEST");
        };
        if envelope.plugin_id != PLUGIN
            || envelope.scene_id.is_empty()
            || envelope.capability_id != CAPABILITY
            || !envelope.command_id.is_null()
            || !envelope
                .input
                .as_object()
                .is_some_and(|input| input.is_empty())
            || envelope.resource_input_profile != "resource.wasi.input.draft2"
            || envelope.resource_output_profile.is_some()
            || resource.index != 0
            || resource.resource_id.is_empty()
            || resource.mime_type != "text/plain"
            || resource.content_version.len() != 64
            || !resource
                .content_version
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("INVALID_REQUEST");
        }
        if resource.byte_length > MAX_SNAPSHOT_BYTES {
            return Err("INPUT_TOO_LARGE");
        }
        zi_resource_input_wasi::read_prefix(0, resource.byte_length, MAX_SNAPSHOT_BYTES)
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod transport {
    use super::*;

    pub(super) fn snapshot() -> Result<Vec<u8>, &'static str> {
        read_stdin(MAX_SNAPSHOT_BYTES)
    }
}
