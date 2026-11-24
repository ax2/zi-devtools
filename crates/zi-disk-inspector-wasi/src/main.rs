use std::io::Read;
use zi_disk_inspector_wasi::{
    MAX_ANALYSIS_BYTES, MAX_HOST_BYTES, MAX_REPORT_BYTES, Report, analyze,
};

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
    let output = transport::execute().and_then(|report| {
        let result = serde_json::to_vec(&report).map_err(|_| "OUTPUT_FAILED")?;
        if result.len() > MAX_REPORT_BYTES {
            return Err("RESULT_TOO_LARGE");
        }
        Ok(result)
    });
    let output = output.unwrap_or_else(|code| {
        let message = match code {
            "INVALID_DIRECTORY_SNAPSHOT" => {
                "The approved directory snapshot is invalid. 无法读取目录用量快照。"
            }
            "INPUT_TOO_LARGE" => "The directory snapshot exceeds 2 MiB. 目录用量快照超过 2 MiB。",
            _ => "Directory analysis could not finish. 目录空间分析未完成。",
        };
        serde_json::to_vec(&serde_json::json!({"ok":false,"error":{"code":code,"message":message}}))
            .expect("serialize error")
    });
    std::io::Write::write_all(&mut std::io::stdout().lock(), &output).expect("write report");
}

#[cfg(target_arch = "wasm32")]
mod transport {
    use super::*;
    use serde::Deserialize;
    use serde_json::Value;
    use zi_disk_inspector_wasi::{CAPABILITY, PLUGIN};

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Envelope {
        plugin_id: String,
        scene_id: String,
        capability_id: String,
        command_id: Value,
        input: Value,
        resource_input_profile: String,
        #[serde(default)]
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

    pub(super) fn execute() -> Result<Report, &'static str> {
        let envelope: Envelope =
            serde_json::from_slice(&read_stdin(16 * 1024)?).map_err(|_| "INVALID_REQUEST")?;
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
        {
            return Err("INVALID_REQUEST");
        }
        let [resource] = envelope.resources.as_slice() else {
            return Err("INVALID_REQUEST");
        };
        if resource.index != 0
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
        if resource.byte_length > MAX_HOST_BYTES {
            return Err("INPUT_TOO_LARGE");
        }
        let bytes =
            zi_resource_input_wasi::read_prefix(0, resource.byte_length, MAX_ANALYSIS_BYTES)?;
        analyze(&bytes, resource.byte_length)
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod transport {
    use super::*;

    pub(super) fn execute() -> Result<Report, &'static str> {
        let bytes = read_stdin(MAX_HOST_BYTES)?;
        analyze(&bytes[..bytes.len().min(MAX_ANALYSIS_BYTES)], bytes.len())
    }
}
