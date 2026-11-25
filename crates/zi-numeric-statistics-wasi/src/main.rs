use serde::Deserialize;
use serde_json::{Value as Json, json, value::RawValue};
use std::io::{Read, Write};
use zi_matrix_core::{Value, parse_real_literal, statistics};

const PLUGIN: &str = "com.zicode.devtools.numeric-statistics";
const CONTRACT: &str = "1.0.0-rc.1";
const REQUEST_LIMIT: usize = 16 * 1024;
const INPUT_LIMIT: usize = 4096;
const VALUE_LIMIT: usize = 64;
const CELL_LIMIT: usize = 96;
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

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Precision {
    #[default]
    Exact,
    Approximate,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    values: String,
    #[serde(default)]
    precision: Precision,
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
    let operation = match request.capability_id.as_str() {
        "devtools.statistics.sum" => "sum",
        "devtools.statistics.mean" => "mean",
        "devtools.statistics.median" => "median",
        "devtools.statistics.population_variance" => "varp",
        "devtools.statistics.sample_variance" => "vars",
        "devtools.statistics.population_standard_deviation" => "stdp",
        "devtools.statistics.sample_standard_deviation" => "stds",
        _ => return Err(Failure("UNKNOWN_OPERATION", "未知统计操作。".into())),
    };
    let input: Input = serde_json::from_str(request.input.get())
        .map_err(|_| Failure("INVALID_INPUT", "请检查数值和精度参数。".into()))?;
    if input.values.len() > INPUT_LIMIT {
        return Err(Failure(
            "INPUT_TOO_LARGE",
            "输入最多 4096 UTF-8 字节。".into(),
        ));
    }
    let text = input.values.strip_suffix('\n').unwrap_or(&input.values);
    let mut values = Vec::new();
    for (index, cell) in text.split([',', '\t', '\n']).enumerate() {
        if index == VALUE_LIMIT {
            return Err(Failure("INVALID_COUNT", "每次最多输入 64 个数值。".into()));
        }
        let cell = cell.trim();
        if cell.is_empty() {
            return Err(Failure(
                "INVALID_VALUE",
                format!("第 {} 个数值为空。", index + 1),
            ));
        }
        if cell.len() > CELL_LIMIT {
            return Err(Failure(
                "INPUT_TOO_LARGE",
                format!("第 {} 个数值最多 96 UTF-8 字节。", index + 1),
            ));
        }
        let value = parse_real_literal(cell, matches!(input.precision, Precision::Approximate))
            .map_err(|error| {
                Failure("INVALID_VALUE", format!("第 {} 个数值：{error}", index + 1))
            })?;
        values.push(value);
    }
    let value = statistics::evaluate(operation, &values)
        .expect("operation above is a known statistic")
        .map_err(|error| Failure("STATISTICS_OPERATION", error))?;
    Ok(
        json!({"count":values.len(),"approximate":matches!(value, Value::Approx(_)),
        "value":value,"text":value.display()}),
    )
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
    let result = match run() {
        Ok(data) => json!({"contractVersion":CONTRACT,"ok":true,"data":data,"error":null}),
        Err(reason) => error(reason),
    };
    let mut bytes = serde_json::to_vec(&result).expect("valid statistics result");
    if bytes.len() > RESULT_LIMIT {
        bytes = serde_json::to_vec(&error(Failure(
            "RESULT_TOO_LARGE",
            "统计结果超过 1 KiB。".into(),
        )))
        .expect("fixed statistics error");
    }
    if std::io::stdout().write_all(&bytes).is_err() {
        std::process::exit(1);
    }
}
