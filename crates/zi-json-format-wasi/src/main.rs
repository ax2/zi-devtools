use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Read;

const MAX_FILE: usize = 1024 * 1024;
type Failure = &'static str;

enum Operation {
    Format,
    Minify,
    Validate,
}
impl Operation {
    fn parse(id: &str, input: Value) -> Result<Self, Failure> {
        if !input.as_object().is_some_and(|fields| fields.is_empty()) {
            return Err("INVALID_REQUEST");
        }
        match id {
            "devtools.json-file.format" => Ok(Self::Format),
            "devtools.json-file.minify" => Ok(Self::Minify),
            "devtools.json-file.validate" => Ok(Self::Validate),
            _ => Err("INVALID_REQUEST"),
        }
    }
    fn count(&self) -> usize {
        1
    }
    fn run(&self, files: &[Vec<u8>]) -> Result<(Value, String), Failure> {
        if files.len() != 1 {
            return Err("INVALID_REQUEST");
        }
        if files[0].len() > MAX_FILE {
            return Err("INPUT_TOO_LARGE");
        }
        let input = std::str::from_utf8(&files[0]).map_err(|_| "INVALID_UTF8")?;
        use zi_text_core::json_file::{Action, run};
        let action = match self {
            Self::Format => Action::Format,
            Self::Minify => Action::Minify,
            Self::Validate => Action::Validate,
        };
        let output = run(input, action)?;
        let report = json!({"ok":true,"schemaVersion":1,"material":"json-file-result",
            "sourceByteCounts":[files[0].len()],"outputByteCount":output.len()});
        Ok((report, output))
    }
}

fn request<T: for<'de> Deserialize<'de>>(limit: usize) -> Result<T, Failure> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "INVALID_REQUEST")?;
    if bytes.len() > limit {
        return Err("INVALID_REQUEST");
    }
    serde_json::from_slice(&bytes).map_err(|_| "INVALID_REQUEST")
}

fn main() {
    let reply = transport::execute().unwrap_or_else(|code| {
        let message = match code {
            "INVALID_JSON" => "JSON syntax is invalid. JSON 语法错误。",
            "JSON_POLICY_REJECTED" => "This tool requires unique object keys, at most 64 nesting levels, safe integers and lossless decimal round trips; no output was created. 本工具要求对象键唯一、最多64层、安全整数及十进制精确往返；未生成结果。这不代表 JSON 语法错误。",
            "INVALID_UTF8" => "The file is not valid UTF-8. 文件不是有效的 UTF-8。",
            "INPUT_TOO_LARGE" => "The file exceeds 1 MiB. 文件超过 1 MiB。",
            "RESULT_TOO_LARGE" => "The complete output exceeds 8 MiB; no output was created. 完整结果超过8 MiB，未生成结果。",
            _ => "The request could not be completed; no output was created. 无法完成请求，未生成结果。",
        };
        json!({"ok":false,"error":{"code":code,"message":message}})
    });
    serde_json::to_writer(std::io::stdout().lock(), &reply).expect("write result");
}

#[cfg(target_arch = "wasm32")]
mod transport {
    use super::*;
    const PLUGIN: &str = "com.zicode.devtools.json-file";
    const CHUNK: usize = 64 * 1024;

    #[link(wasm_import_module = "zicode_resource_output_draft1")]
    unsafe extern "C" {
        #[link_name = "create"]
        fn output_create(mime_index: i32, length: i64) -> i32;
        #[link_name = "write"]
        fn output_write(index: i32, source: *const u8, length: i32) -> i32;
        #[link_name = "finish"]
        fn output_finish(index: i32) -> i32;
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Envelope {
        plugin_id: String,
        scene_id: String,
        capability_id: String,
        command_id: Value,
        input: Value,
        resource_input_profile: String,
        resource_output_profile: String,
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

    fn read_file(index: usize, file: &Descriptor) -> Result<Vec<u8>, Failure> {
        if file.index != index
            || file.resource_id.is_empty()
            || file.content_version.len() != 64
            || !file
                .content_version
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || !matches!(
                file.mime_type.as_str(),
                "text/plain" | "application/octet-stream" | "application/json"
            )
        {
            return Err("INVALID_REQUEST");
        }
        if file.byte_length > MAX_FILE {
            return Err("INPUT_TOO_LARGE");
        }
        zi_resource_input_wasi::read_prefix(index as i32, file.byte_length, file.byte_length)
    }

    pub(super) fn execute() -> Result<Value, Failure> {
        let envelope: Envelope = request(32 * 1024)?;
        if envelope.plugin_id != PLUGIN
            || envelope.scene_id.is_empty()
            || !envelope.command_id.is_null()
            || envelope.resource_input_profile != "resource.wasi.input.draft2"
            || envelope.resource_output_profile != "resource.wasi.output.draft1"
        {
            return Err("INVALID_REQUEST");
        }
        let operation = Operation::parse(&envelope.capability_id, envelope.input)?;
        if envelope.resources.len() != operation.count() {
            return Err("INVALID_REQUEST");
        }
        let files = envelope
            .resources
            .iter()
            .enumerate()
            .map(|(index, file)| read_file(index, file))
            .collect::<Result<Vec<_>, _>>()?;
        let (report, output) = operation.run(&files)?;
        // Reserved application/json index for the JSON-file compare execution profile.
        let index = unsafe { output_create(28, output.len() as i64) };
        if index != 0 {
            return Err("OUTPUT_FAILED");
        }
        for chunk in output.as_bytes().chunks(CHUNK) {
            if unsafe { output_write(index, chunk.as_ptr(), chunk.len() as i32) }
                != chunk.len() as i32
            {
                return Err("OUTPUT_FAILED");
            }
        }
        if unsafe { output_finish(index) } != 0 {
            return Err("OUTPUT_FAILED");
        }
        Ok(report)
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod transport {
    use super::*;
    use base64::{Engine, engine::general_purpose::STANDARD};

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Fixture {
        capability_id: String,
        input: Value,
        files_base64: Vec<String>,
    }

    pub(super) fn execute() -> Result<Value, Failure> {
        let fixture: Fixture = request(((MAX_FILE + 1).div_ceil(3) * 4) * 2 + 32 * 1024)?;
        let operation = Operation::parse(&fixture.capability_id, fixture.input)?;
        if fixture.files_base64.len() != operation.count() {
            return Err("INVALID_REQUEST");
        }
        let files = fixture
            .files_base64
            .iter()
            .map(|value| STANDARD.decode(value).map_err(|_| "INVALID_REQUEST"))
            .collect::<Result<Vec<_>, _>>()?;
        let (report, output) = operation.run(&files)?;
        Ok(json!({"report":report,"outputBase64":STANDARD.encode(output)}))
    }
}
