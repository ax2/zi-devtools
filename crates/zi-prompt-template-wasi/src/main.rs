//! Uses ZiDevTools' local prompt-placeholder rules without a library or model.
use serde::Deserialize;
use serde_json::{Value as Json, json, value::RawValue};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
};
use zi_prompt_template_core::{MAX_VARIABLES, parse};

const PLUGIN: &str = "com.zicode.devtools.prompt-expansion";
const INSPECT: &str = "devtools.prompt.variables";
const EXPAND: &str = "devtools.prompt.expand";
const CONTRACT: &str = "1.0.0-rc.1";
const REQUEST_LIMIT: usize = 48 * 1024;
const RESULT_LIMIT: usize = 48 * 1024;
const PREVIEW_LIMIT: usize = 16 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Request<'a> {
    plugin_id: String,
    scene_id: String,
    capability_id: String,
    command_id: Json,
    #[serde(borrow)]
    input: &'a RawValue,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InspectInput {
    template: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpandInput {
    template: String,
    #[serde(default)]
    values: BTreeMap<String, String>,
}

struct Failure(&'static str, String);

fn execute(request: Request<'_>) -> Result<Json, Failure> {
    if request.plugin_id != PLUGIN
        || request.scene_id.is_empty()
        || request.scene_id.len() > 128
        || !request.command_id.is_null()
    {
        return Err(Failure("INVALID_REQUEST", "请求结构无效。".into()));
    }
    let invalid_input = |_| Failure("INVALID_INPUT", "请输入模板正文和文本变量值。".into());
    let template_error = |error: anyhow::Error| Failure("TEMPLATE_INVALID", error.to_string());
    match request.capability_id.as_str() {
        INSPECT => {
            let input: InspectInput =
                serde_json::from_str(request.input.get()).map_err(invalid_input)?;
            let template = parse(&input.template).map_err(template_error)?;
            let names = template.variable_names();
            let text = if names.is_empty() {
                "此模板没有变量。".into()
            } else {
                format!("变量 {} 项\n{}", names.len(), names.join("\n"))
            };
            Ok(json!({"variables":names,"templateByteLength":input.template.len(),"text":text}))
        }
        EXPAND => {
            let input: ExpandInput =
                serde_json::from_str(request.input.get()).map_err(invalid_input)?;
            if input.values.len() > MAX_VARIABLES {
                return Err(Failure("INVALID_INPUT", "最多提供 12 个变量值。".into()));
            }
            let template = parse(&input.template).map_err(template_error)?;
            let expanded = template
                .render(&input.values)
                .map_err(|error| Failure("EXPANSION_FAILED", error.to_string()))?;
            Ok(expansion_report(&expanded, template.variable_names()))
        }
        _ => Err(Failure(
            "UNSUPPORTED_OPERATION",
            "请选择变量检查或模板展开。".into(),
        )),
    }
}

fn expansion_report(expanded: &str, names: &[String]) -> Json {
    // Reserve metadata and the optional visible truncation notice. JSON uses up
    // to six bytes for an ASCII control character without changing the text.
    let mut escaped = 0;
    let cap = PREVIEW_LIMIT.min(expanded.len());
    let mut end = cap;
    for (position, &byte) in expanded.as_bytes()[..cap].iter().enumerate() {
        escaped += match byte {
            b'"' | b'\\' | b'\n' | b'\r' | b'\t' | 8 | 12 => 2,
            0..=31 => 6,
            _ => 1,
        };
        if escaped > RESULT_LIMIT - 2048 {
            end = position;
            break;
        }
    }
    while !expanded.is_char_boundary(end) {
        end -= 1;
    }
    let mut text = expanded[..end].to_owned();
    let truncated = end < expanded.len();
    if truncated {
        use std::fmt::Write as _;
        let _ = write!(
            text,
            "\n\n[预览显示前 {end}/{} 字节，已省略剩余内容]",
            expanded.len()
        );
    }
    json!({"variables":names,"expandedByteLength":expanded.len(),"previewByteLength":end,
           "truncated":truncated,"text":text})
}

fn run() -> Result<Json, Failure> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take((REQUEST_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure("INVALID_REQUEST", "无法读取请求。".into()))?;
    if bytes.len() > REQUEST_LIMIT {
        return Err(Failure("INPUT_TOO_LARGE", "请求超过 48 KiB。".into()));
    }
    let request = serde_json::from_slice(&bytes)
        .map_err(|_| Failure("INVALID_REQUEST", "请求结构无效。".into()))?;
    execute(request)
}

fn error(Failure(code, message): Failure) -> Json {
    json!({"contractVersion":CONTRACT,"ok":false,"data":null,"error":{"code":code,"message":message}})
}

fn main() {
    let outcome = match run() {
        Ok(data) => json!({"contractVersion":CONTRACT,"ok":true,"data":data,"error":null}),
        Err(reason) => error(reason),
    };
    let mut bytes = serde_json::to_vec(&outcome).expect("valid prompt result");
    if bytes.len() > RESULT_LIMIT {
        bytes = serde_json::to_vec(&error(Failure(
            "RESULT_TOO_LARGE",
            "预览结果超过 48 KiB。".into(),
        )))
        .expect("fixed preview error");
    }
    if std::io::stdout().write_all(&bytes).is_err() {
        std::process::exit(1);
    }
}
