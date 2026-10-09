//! Experimental proposal.2 adapter for four complete JSON/regex operations.
//! Not frozen rc.1, not a Host-negotiated plugin, and not the full five-operation draft.
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

pub const CONTRACT_VERSION: &str = "1.1.0-proposal.2";
pub const REQUEST_LIMIT: usize = 48 * 1024;
pub const OPERATIONS: [&str; 4] = [
    "devtools.compare.json.path",
    "devtools.compare.json.diff.ordered",
    "devtools.compare.json.diff.unordered",
    "devtools.compare.regex.matches",
];

#[derive(Deserialize)]
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
struct PathInput {
    text: String,
    query: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DiffInput {
    left: String,
    right: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegexInput {
    text: String,
    pattern: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ResultEnvelope {
    contract_version: &'static str,
    ok: bool,
    data: Option<Text>,
    error: Option<Error>,
}
#[derive(Serialize)]
struct Text {
    text: String,
}
#[derive(Serialize)]
struct Error {
    code: &'static str,
    message: &'static str,
}
impl ResultEnvelope {
    fn failure(code: &'static str, message: &'static str) -> Self {
        Self {
            contract_version: CONTRACT_VERSION,
            ok: false,
            data: None,
            error: Some(Error { code, message }),
        }
    }
    fn success(text: String) -> Self {
        Self {
            contract_version: CONTRACT_VERSION,
            ok: true,
            data: Some(Text { text }),
            error: None,
        }
    }
}

fn handle(bytes: &[u8]) -> ResultEnvelope {
    if bytes.len() > REQUEST_LIMIT {
        return ResultEnvelope::failure("INPUT_TOO_LARGE", "请求超过 48 KiB");
    }
    if std::str::from_utf8(bytes).is_err() {
        return ResultEnvelope::failure("INVALID_ENCODING", "请求不是有效 UTF-8");
    }
    let Ok(request) = serde_json::from_slice::<Request>(bytes) else {
        return ResultEnvelope::failure("INVALID_INPUT", "请求结构无效");
    };
    // Required explicit null: no commands or implicit default handlers.
    let () = request.command_id;
    if request.contract_version != CONTRACT_VERSION
        || request.plugin_id != "com.zicode.devtools.compare"
        || request.scene_id.is_empty()
        || request.scene_id.len() > 128
    {
        return ResultEnvelope::failure("INVALID_INPUT", "协议、插件或场景标识无效");
    }
    let result = match request.capability_id.as_str() {
        "devtools.compare.regex.matches" => {
            let Ok(input) = serde_json::from_str::<RegexInput>(request.input.get()) else {
                return ResultEnvelope::failure("INVALID_INPUT", "正则输入结构无效");
            };
            if input.text.len() > 8192 || input.pattern.len() > 4096 {
                return ResultEnvelope::failure("INPUT_TOO_LARGE", "正则字段超过 UTF-8 字节限制");
            }
            zi_regex_core::test_regex_with_result_budget(&input.pattern, &input.text, REQUEST_LIMIT)
        }
        "devtools.compare.json.path" => {
            let Ok(input) = serde_json::from_str::<PathInput>(request.input.get()) else {
                return ResultEnvelope::failure("INVALID_INPUT", "查询输入结构无效");
            };
            if input.text.len() > 8192 || input.query.len() > 4096 {
                return ResultEnvelope::failure("INPUT_TOO_LARGE", "查询字段超过 UTF-8 字节限制");
            }
            zi_json_core::json_path(&input.text, &input.query)
        }
        "devtools.compare.json.diff.ordered" | "devtools.compare.json.diff.unordered" => {
            let Ok(input) = serde_json::from_str::<DiffInput>(request.input.get()) else {
                return ResultEnvelope::failure("INVALID_INPUT", "差异输入结构无效");
            };
            if input.left.len() > 8192 || input.right.len() > 8192 {
                return ResultEnvelope::failure("INPUT_TOO_LARGE", "差异字段超过 UTF-8 字节限制");
            }
            zi_json_core::json_diff_with_result_budget(
                &input.left,
                &input.right,
                request.capability_id.ends_with(".unordered"),
                REQUEST_LIMIT,
            )
        }
        _ => return ResultEnvelope::failure("UNSUPPORTED_OPERATION", "该实验适配未注册此能力"),
    };
    match result {
        Ok(text) => ResultEnvelope::success(text),
        Err(error)
            if error.is::<zi_json_core::ReportTooLarge>()
                || error.is::<zi_regex_core::ReportTooLarge>() =>
        {
            ResultEnvelope::failure("INPUT_TOO_LARGE", "结果超过 48 KiB")
        }
        Err(_) => ResultEnvelope::failure("INVALID_INPUT", "输入无法按该操作处理"),
    }
}

/// One exact serialized result. Both success and failure obey the byte budget.
pub fn execute_request(bytes: &[u8]) -> Vec<u8> {
    let result = handle(bytes);
    let output =
        serde_json::to_vec(&result).expect("result contains only JSON strings and booleans");
    if output.len() <= REQUEST_LIMIT {
        output
    } else {
        serde_json::to_vec(&ResultEnvelope::failure(
            "INPUT_TOO_LARGE",
            "结果超过 48 KiB",
        ))
        .expect("fixed error serializes")
    }
}

#[cfg(test)]
mod tests;
