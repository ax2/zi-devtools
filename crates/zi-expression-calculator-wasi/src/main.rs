use serde::Deserialize;
use serde_json::{Value as Json, json, value::RawValue};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
};
use zi_matrix_core::expression::{
    evaluate,
    input::{AngleInput, Literal, parse_constants},
};

const PLUGIN: &str = "com.zicode.devtools.expression-calculator";
const CAPABILITY: &str = "devtools.calculator.evaluate";
const CONTRACT: &str = "1.0.0-rc.1";
const REQUEST_LIMIT: usize = 16 * 1024;
const RESULT_LIMIT: usize = 1024;

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
struct Input {
    formula: String,
    #[serde(default)]
    angle: AngleInput,
    #[serde(default)]
    constants: BTreeMap<String, Literal>,
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
    if request.capability_id != CAPABILITY {
        return Err(Failure("UNKNOWN_OPERATION", "未知表达式操作。".into()));
    }
    let input: Input = serde_json::from_str(request.input.get())
        .map_err(|_| Failure("INVALID_INPUT", "请检查公式和计算参数。".into()))?;
    let variables = parse_constants(input.constants, false)
        .map_err(|reason| Failure("INVALID_CONSTANT", reason))?;
    let value = evaluate(&input.formula, &variables, input.angle.into())
        .map_err(|reason| Failure("EXPRESSION_OPERATION", reason))?;
    Ok(json!({"value":value,"approximate":value.is_approximate(),"text":value.display()}))
}

fn run() -> Result<Json, Failure> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take((REQUEST_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure("INVALID_REQUEST", "无法读取请求。".into()))?;
    if bytes.len() > REQUEST_LIMIT {
        return Err(Failure("INPUT_TOO_LARGE", "请求超过 16 KiB。".into()));
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
    let mut bytes = serde_json::to_vec(&outcome).expect("valid calculator result");
    if bytes.len() > RESULT_LIMIT {
        bytes = serde_json::to_vec(&error(Failure(
            "RESULT_TOO_LARGE",
            "计算结果超过 1 KiB。".into(),
        )))
        .expect("fixed calculator error");
    }
    if std::io::stdout().write_all(&bytes).is_err() {
        std::process::exit(1);
    }
}
