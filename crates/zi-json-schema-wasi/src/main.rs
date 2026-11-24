use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{Read, Write};
use zi_json_schema_core::validate_legacy_document;

const REQUEST_LIMIT: usize = 48 * 1024;
const FIELD_LIMIT: usize = 8 * 1024;
const RESULT_LIMIT: usize = 48 * 1024;
const PLUGIN: &str = "com.zicode.devtools.json-schema";
const CAPABILITY: &str = "devtools.json-schema.validate";
const CONTRACT: &str = "1.0.0-rc.1";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    plugin_id: String,
    capability_id: String,
    scene_id: String,
    command_id: Value,
    input: Input,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    instance: String,
    schema: String,
}

fn failure(code: &str, message: &str) -> Value {
    json!({"contractVersion":CONTRACT,"ok":false,"data":null,"error":{"code":code,"message":message}})
}

fn execute(bytes: &[u8]) -> Value {
    if bytes.len() > REQUEST_LIMIT {
        return failure(
            "INPUT_TOO_LARGE",
            "请求超过 48 KiB。Request exceeds 48 KiB.",
        );
    }
    let Ok(request) = serde_json::from_slice::<Request>(bytes) else {
        return failure("INVALID_INPUT", "请求格式无效。Malformed request.");
    };
    if request.plugin_id != PLUGIN
        || request.capability_id != CAPABILITY
        || request.scene_id.is_empty()
        || !request.command_id.is_null()
    {
        return failure("INVALID_INPUT", "请求标识无效。Invalid request identity.");
    }
    if request.input.instance.len() > FIELD_LIMIT || request.input.schema.len() > FIELD_LIMIT {
        return failure(
            "INPUT_TOO_LARGE",
            "实例与 Schema 每项最多 8 KiB。Each input is limited to 8 KiB.",
        );
    }
    match validate_legacy_document(&request.input.instance, &request.input.schema) {
        Ok(text) => json!({"contractVersion":CONTRACT,"ok":true,"data":{"text":text},"error":null}),
        Err(error) => failure("VALIDATION_FAILED", &error.to_string()),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = Vec::new();
    std::io::stdin()
        .take((REQUEST_LIMIT + 1) as u64)
        .read_to_end(&mut input)?;
    let output = serde_json::to_vec(&execute(&input))?;
    let mut stdout = std::io::stdout().lock();
    if output.len() <= RESULT_LIMIT {
        stdout.write_all(&output)?;
    } else {
        serde_json::to_writer(
            stdout,
            &failure(
                "RESULT_TOO_LARGE",
                "校验报告超过 48 KiB。Validation report exceeds 48 KiB.",
            ),
        )?;
    }
    Ok(())
}
