//! Explicit clock proposal experiment, not a negotiated or installed plugin.
//! Four operations reuse the existing adapter in process. No old wire fallback.
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

pub const CONTRACT_VERSION: &str = "1.1.0-proposal.3";
pub const REQUEST_LIMIT: usize = 48 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    contract_version: String,
    plugin_id: String,
    scene_id: String,
    capability_id: String,
    command_id: (),
    input: Box<RawValue>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextInput {
    left: String,
    right: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Envelope {
    contract_version: String,
    ok: bool,
    data: Option<Text>,
    error: Option<Error>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Text {
    text: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Error {
    code: String,
    message: String,
}

fn failure(code: &str, message: &str) -> Envelope {
    Envelope {
        contract_version: CONTRACT_VERSION.to_owned(),
        ok: false,
        data: None,
        error: Some(Error {
            code: code.to_owned(),
            message: message.to_owned(),
        }),
    }
}

fn clock_available() -> bool {
    #[cfg(target_os = "wasi")]
    {
        // The generated binding owns the ABI and output pointer. A host must keep
        // this clock available for the whole invocation, not just the preflight.
        unsafe { wasi::clock_time_get(wasi::CLOCKID_MONOTONIC, 1).is_ok() }
    }
    #[cfg(not(target_os = "wasi"))]
    {
        true
    }
}

fn handle(bytes: &[u8]) -> Envelope {
    if bytes.len() > REQUEST_LIMIT {
        return failure("INPUT_TOO_LARGE", "请求超过 48 KiB");
    }
    if std::str::from_utf8(bytes).is_err() {
        return failure("INVALID_ENCODING", "请求不是有效 UTF-8");
    }
    let Ok(mut request) = serde_json::from_slice::<Request>(bytes) else {
        return failure("INVALID_INPUT", "请求结构无效");
    };
    if request.contract_version != CONTRACT_VERSION
        || request.plugin_id != "com.zicode.devtools.compare"
        || request.scene_id.is_empty()
        || request.scene_id.len() > 128
    {
        return failure("INVALID_INPUT", "协议、插件或场景标识无效");
    }
    if request.capability_id == "devtools.compare.text.diff" {
        let Ok(input) = serde_json::from_str::<TextInput>(request.input.get()) else {
            return failure("INVALID_INPUT", "差异输入结构无效");
        };
        if input.left.len() > 8192 || input.right.len() > 8192 {
            return failure("INPUT_TOO_LARGE", "差异字段超过 UTF-8 字节限制");
        }
        if !clock_available() {
            return failure("RUNTIME_UNAVAILABLE", "运行环境未提供单调时钟");
        }
        return match zi_diff_core::compare_text(&input.left, &input.right, Some(REQUEST_LIMIT)) {
            Ok(text) => Envelope {
                contract_version: CONTRACT_VERSION.to_owned(),
                ok: true,
                data: Some(Text { text }),
                error: None,
            },
            Err(zi_diff_core::CompareError::ReportTooLarge) => {
                failure("INPUT_TOO_LARGE", "结果超过 48 KiB")
            }
            Err(zi_diff_core::CompareError::InputTooLarge) => {
                failure("INVALID_INPUT", "输入无法按该操作处理")
            }
        };
    }
    // Reuse the complete four-operation implementation as a Rust library.
    // The public entry accepts only proposal.3; this is internal normalization,
    // not negotiation, a Host call, an executable or a fallback wire protocol.
    request.contract_version = zi_fields_wasi::CONTRACT_VERSION.to_owned();
    let normalized = serde_json::to_vec(&request).expect("request JSON serializes");
    let mut result: Envelope =
        serde_json::from_slice(&zi_fields_wasi::execute_request(&normalized))
            .expect("existing adapter produces JSON");
    result.contract_version = CONTRACT_VERSION.to_owned();
    result
}

/// Bound the final escaped JSON envelope as well as the report itself.
pub fn execute_request(bytes: &[u8]) -> Vec<u8> {
    let output = serde_json::to_vec(&handle(bytes)).expect("result JSON serializes");
    if output.len() <= REQUEST_LIMIT {
        output
    } else {
        serde_json::to_vec(&failure("INPUT_TOO_LARGE", "结果超过 48 KiB"))
            .expect("fixed error serializes")
    }
}

#[cfg(test)]
mod tests;
